use crate::map::{CELL, MAP};
use crate::model::{Rig, barrel_length, spawn_soldier, spawn_weapon, viewmodel_offset};
use crate::weapons::{ALL_WEAPONS, DRAW_SECS, Weapon, buy_menu};
use crate::shared::*;
use crate::sfx;
use bevy::audio::Volume;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::winit::WinitSettings;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const MOUSE_SENS: f32 = 0.002;
const REDUNDANCY: usize = 8;
/// Field of view, and scoped in with the AWP.
const FOV: f32 = 1.6;
const ZOOM_FOV: f32 = 0.42;
const HUD_AMBER: Color = Color::srgba(1.0, 0.68, 0.12, 0.9);
const CROSSHAIR_GREEN: Color = Color::srgba(0.25, 1.0, 0.25, 0.85);

#[derive(Resource)]
struct Net {
    sock: UdpSocket,
    server: SocketAddr,
    lag: Duration,
    /// Extra random delay on packets both ways, kept in order, to mimic bursty Wi-Fi.
    jitter: Duration,
    rng: u32,
    out_q: VecDeque<(Instant, Vec<u8>)>,
    in_q: VecDeque<(Instant, Vec<u8>)>,
}

impl Net {
    fn send(&mut self, data: Vec<u8>) {
        let at = self.fake_arrival(true);
        self.out_q.push_back((at, data));
        self.flush();
    }

    fn fake_arrival(&mut self, outgoing: bool) -> Instant {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let at = Instant::now() + self.lag + self.jitter.mul_f64(self.rng as f64 / u32::MAX as f64);
        let q = if outgoing { &self.out_q } else { &self.in_q };
        q.back().map_or(at, |(last, _)| at.max(*last))
    }

    fn flush(&mut self) {
        while self.out_q.front().is_some_and(|(t, _)| *t <= Instant::now()) {
            let (_, data) = self.out_q.pop_front().unwrap();
            let _ = self.sock.send_to(&data, self.server);
        }
    }

    fn recv(&mut self) -> Vec<Vec<u8>> {
        let mut buf = [0u8; 65536];
        while let Ok(n) = self.sock.recv(&mut buf) {
            let at = self.fake_arrival(false);
            self.in_q.push_back((at, buf[..n].to_vec()));
        }
        let mut out = vec![];
        while self.in_q.front().is_some_and(|(t, _)| *t <= Instant::now()) {
            out.push(self.in_q.pop_front().unwrap().1);
        }
        out
    }
}

#[derive(Resource, Default)]
struct Game {
    me: Option<u32>,
    seq: u32,
    pred: PlayerState,
    prev_pos: Vec3,
    /// Visual offset left over from reconciliation, decayed to zero so corrections don't snap.
    smooth: Vec3,
    pending: VecDeque<Input>,
    /// Network round trip only, like CS: excludes tick and queue waits on either side.
    ping_ms: f32,
    hp: i32,
    kills: u32,
    deaths: u32,
    latest_tick: u32,
    /// Server tick minus local clock in ticks. Estimating server time as local clock + offset keeps it advancing
    /// smoothly between snapshots instead of lurching each time one arrives.
    tick_offset: Option<f64>,
    snapshots: VecDeque<Snapshot>,
    yaw: f32,
    pitch: f32,
    /// Bullet tracers: (from, to, seconds left, lifetime, color).
    tracers: Vec<(Vec3, Vec3, f32, f32, Color)>,
    /// How far behind the newest snapshot remote players are drawn. Adapts to jitter, never below `min_interp_ticks`.
    interp_ticks: f64,
    min_interp_ticks: f64,
    /// Recent worst gap between snapshots in ticks, decaying slowly.
    jitter_ticks: f64,
    net_graph: NetGraph,
    /// Got packets we couldn't decode: the server runs a different build.
    incompatible: bool,
    /// The server rejected us: it runs this other build.
    wrong_build: Option<String>,
    /// Sounds to play this frame, with volume.
    sounds: Vec<(Sound, f32)>,
    died_at: Option<Instant>,
    round: Option<Round>,
    teams: HashMap<u32, Team>,
    /// Clock time of the next bomb beep, and of the last one (for the light).
    next_beep: f64,
    last_beep: f64,
    /// Where and when (clock) the bomb went off, for the flash.
    explosion: Option<(Vec3, f64)>,
    /// The teammate we watch while dead: id, interpolated eye position and view.
    spectate: Option<(u32, Vec3, f32, f32)>,
    /// The player under the crosshair, if any.
    aim_target: Option<u32>,
    /// Interpolated position of a remote bomb carrier.
    carrier_pos: Option<Vec3>,
    armor: u8,
    money: u32,
    names: HashMap<u32, String>,
    /// Scoped in with the AWP.
    zoom: bool,
    /// Buy menu open.
    menu: bool,
    /// Radio menu (Z) open.
    radio_menu: bool,
    want_radio: u8,
    /// Key presses caught each frame, sent with the next input.
    want_slot: u8,
    want_buy: u8,
    want_reload: bool,
    kill_feed: VecDeque<KillLine>,
    /// Bumped whenever the kill feed changes, so its rows are rebuilt only then.
    feed_version: u32,
    hurt_at: Option<Instant>,
    fx: Vec<FxKind>,
    /// Everyone as of the newest snapshot, for the scoreboard.
    players: Vec<NetPlayer>,
    /// Distance run since the last footstep.
    stride: f32,
    /// Radio chat lines and when they were said.
    chat: VecDeque<(String, Instant)>,
    /// Big centered announcement and when it was made.
    announce: Option<(String, Instant)>,
    /// Viewmodel kick from the last shot, 1 right after firing and decaying to 0.
    kick: f32,
    steps_played: u32,
}

#[derive(Clone, Copy)]
enum Sound {
    Gun(Weapon),
    Reload,
    Buy,
    Headshot,
    HitBody,
    Kill,
    Hurt,
    Death,
    Beep,
    Explosion,
    Step(u32),
    Impact,
    Voice(Announce),
    Radio(Radio),
}

/// Radio chat lines, bottom left.
#[derive(Component)]
struct ChatText;

#[derive(Component)]
struct RadioMenu;

/// Announcer lines, CS style.
#[derive(Clone, Copy, PartialEq)]
enum Announce {
    TWin,
    CtWin,
    Planted,
    Defused,
    Go,
    EnemyDown,
}

#[derive(Component)]
struct AnnounceText;

/// "Friend: name  Health: 100%" / "Enemy: name" under the crosshair target, or who we're spectating.
#[derive(Component)]
struct TargetText;

/// A short-lived visual effect to spawn this frame.
#[derive(Clone, Copy)]
enum FxKind {
    Muzzle(Vec3),
    /// Bullet striking the map: dust puff and a hole, at a position with the surface normal.
    Impact(Vec3, Vec3),
    Blood(Vec3),
}

/// A spawned effect: grows and fades over its life, then despawns.
#[derive(Component)]
struct Fx {
    age: f32,
    life: f32,
    /// Size multiplier reached at the end of its life, growing linearly from 1.
    grow: f32,
    size: f32,
    material: Option<Handle<StandardMaterial>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Life {
    Alive,
    /// Shown as a corpse until the round restarts.
    Dead,
    /// Alive again but the respawn teleport hasn't been interpolated past yet: hidden.
    Respawning,
}

/// Bullet holes, oldest first, so only the most recent stay.
#[derive(Resource, Default)]
struct Decals(VecDeque<Entity>);

#[derive(Resource)]
struct Sounds {
    guns: HashMap<Weapon, Handle<AudioSource>>,
    reload: Handle<AudioSource>,
    buy: Handle<AudioSource>,
    headshot: Handle<AudioSource>,
    hit_body: Handle<AudioSource>,
    kill: Handle<AudioSource>,
    hurt: Handle<AudioSource>,
    death: Handle<AudioSource>,
    beep: Handle<AudioSource>,
    explosion: Handle<AudioSource>,
    steps: Vec<Handle<AudioSource>>,
    impact: Handle<AudioSource>,
    voices: Vec<(Announce, Handle<AudioSource>)>,
    radio: Vec<(Radio, Handle<AudioSource>)>,
}

/// First-person model of one weapon.
#[derive(Component)]
struct Viewmodel(Weapon);

#[derive(Component)]
struct AmmoText;

#[derive(Component)]
struct BuyMenu;

#[derive(Component)]
struct KillFeed;

/// One kill feed entry: killer and victim with their teams, the weapon, headshot.
struct KillLine {
    killer: (String, Option<Team>),
    weapon: Weapon,
    victim: (String, Option<Team>),
    headshot: bool,
    at: Instant,
}

#[derive(Component)]
struct BuyRow(usize);

#[derive(Component)]
struct BuyTitle;

#[derive(Component)]
struct HurtFlash;

/// Sniper scope overlay shown while zoomed with the AWP.
#[derive(Component)]
struct Scope;

#[derive(Component)]
struct Scoreboard;

#[derive(Component)]
struct BombModel;

/// The C4's red light, blinking with each beep.
#[derive(Component)]
struct BombLed(Handle<StandardMaterial>);

#[derive(Component)]
struct DeathFade;

/// Worst-case network stats over the last second, shown like CS's net_graph.
#[derive(Default)]
struct NetGraph {
    last_snap: Option<Instant>,
    max_gap_ms: f32,
    max_fix_cm: f32,
    starved: u32,
    frames: u32,
    worst_frame_ms: f32,
    since: Option<Instant>,
    line: String,
}

#[derive(Component)]
struct Remote(u32);

#[derive(Component)]
struct Hud;

/// One of the four crosshair lines: its direction from the center.
#[derive(Component)]
struct Crosshair(Vec2);

#[derive(Component)]
struct HealthText;

#[derive(Component)]
struct TimerText;

#[derive(Component)]
struct ArmorText;

#[derive(Component)]
struct MoneyText;

#[derive(Component)]
struct NetGraphText;

/// The Tab scoreboard: panel and the two team sections.
#[derive(Component)]
struct ScorePanel;

#[derive(Component)]
struct ScoreSection(Team);

pub fn run(server: SocketAddr, lag_ms: u64, jitter_ms: u64, interp_ms: u64) {
    let sock = UdpSocket::bind("0.0.0.0:0").expect("bind");
    sock.set_nonblocking(true).unwrap();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("fps16 build {BUILD}"),
                        present_mode: PresentMode::AutoNoVsync,
                        ..default()
                    }),
                    ..default()
                })
                // Pipelining renders one frame behind the simulation; an extra frame of input lag isn't worth it here.
                .disable::<PipelinedRenderingPlugin>(),
        )
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(WinitSettings::continuous())
        .insert_resource(Net {
            sock,
            server,
            lag: Duration::from_millis(lag_ms),
            jitter: Duration::from_millis(jitter_ms),
            rng: 0x9E37_79B9,
            out_q: default(),
            in_q: default(),
        })
        .insert_resource(Game {
            hp: MAX_HP,
            min_interp_ticks: interp_ms as f64 / 1000.0 * TICK_HZ,
            interp_ticks: interp_ms as f64 / 1000.0 * TICK_HZ,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(PostUpdate, debug_shot.run_if(|| std::env::var_os("FPS16_SHOT").is_some()))
        .add_systems(PreUpdate, receive)
        .add_systems(FixedUpdate, net_tick)
        .add_systems(
            Update,
            (
                grab_cursor,
                look,
                controls,
                (render_remotes, render_self, viewmodel, render_bomb, draw_tracers, bomb_beep, play_sounds, spawn_fx, update_fx),
                (hud, scoreboard, ammo_hud, buy_menu_hud, kill_feed, death_fade, hurt_flash, crosshair, scope, score_panel, announce, chat, radio_menu, target_id),
            )
                .chain(),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut sound = |wav: Vec<u8>| audio.add(AudioSource { bytes: wav.into() });
    commands.insert_resource(Sounds {
        guns: ALL_WEAPONS.into_iter().map(|w| (w, sound(sfx::gunshot(w)))).collect(),
        reload: sound(sfx::reload()),
        buy: sound(sfx::buy()),
        headshot: sound(sfx::headshot()),
        hit_body: sound(sfx::hit_body()),
        kill: sound(sfx::hit()),
        hurt: sound(sfx::hurt()),
        death: sound(sfx::death()),
        beep: sound(sfx::beep()),
        explosion: sound(sfx::explosion()),
        steps: (0..4).map(|v| sound(sfx::footstep(v))).collect(),
        impact: sound(sfx::impact()),
        voices: [
            (Announce::TWin, sfx::VOICE_T_WIN),
            (Announce::CtWin, sfx::VOICE_CT_WIN),
            (Announce::Planted, sfx::VOICE_PLANTED),
            (Announce::Defused, sfx::VOICE_DEFUSED),
            (Announce::Go, sfx::VOICE_GO),
            (Announce::EnemyDown, sfx::VOICE_ENEMY_DOWN),
        ]
        .into_iter()
        .map(|(a, wav)| (a, sound(sfx::radio(wav))))
        .collect(),
        radio: [Radio::EnemySpotted, Radio::NeedBackup, Radio::InPosition, Radio::SectorClear, Radio::CoverMe]
            .into_iter()
            .map(|r| (r, sound(sfx::radio(sfx::radio_line(r)))))
            .collect(),
    });
    commands.insert_resource(Decals::default());
    commands.spawn((
        AudioPlayer::new(audio.add(AudioSource { bytes: sfx::wind().into() })),
        PlaybackSettings { volume: Volume::Linear(0.18), ..PlaybackSettings::LOOP },
    ));
    // First-person weapons; each barrel tip lines up with `muzzle`.
    let viewmodels: Vec<Entity> = ALL_WEAPONS
        .into_iter()
        .map(|w| {
            let e = spawn_weapon(&mut commands, &mut meshes, &mut mats, w, Transform::from_translation(viewmodel_offset(w)), true);
            commands.entity(e).insert(Viewmodel(w)).id()
        })
        .collect();
    // Desert sky, hazy distance, warm hard sunlight.
    commands.insert_resource(ClearColor(Color::srgb(0.5, 0.68, 0.9)));
    let camera = commands
        .spawn((
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection { fov: FOV, ..default() }),
            DistanceFog {
                color: Color::srgb(0.72, 0.76, 0.82),
                directional_light_color: Color::srgba(1.0, 0.9, 0.7, 0.4),
                directional_light_exponent: 30.0,
                falloff: FogFalloff::Linear { start: 140.0, end: 700.0 },
            },
        ))
        .add_children(&viewmodels)
        .id();
    if std::env::var_os("FPS16_SHOT").is_some() {
        // Render off-screen so screenshots work even when the window is hidden behind others.
        use bevy::render::render_resource::TextureFormat;
        let target = images.add(Image::new_target_texture(1600, 900, TextureFormat::Rgba8UnormSrgb, None));
        commands.entity(camera).insert((bevy::camera::RenderTarget::from(target.clone()), IsDefaultUiCamera));
        commands.insert_resource(ShotTarget(target));
    }
    commands.spawn((
        DirectionalLight { illuminance: 16000.0, color: Color::srgb(1.0, 0.93, 0.8), shadow_maps_enabled: true, ..default() },
        Transform::default().looking_to(Vec3::new(-0.5, -0.9, -0.35), Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 350.0, color: Color::srgb(0.8, 0.85, 1.0), ..default() });
    spawn_world(&mut commands, &mut meshes, &mut mats, &mut images);
    let site_mesh = meshes.add(Plane3d::default().mesh().size(CELL, CELL));
    let site_mat = mats.add(StandardMaterial {
        base_color: Color::srgba(0.8, 0.15, 0.1, 0.3),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    for p in MAP.site_cells() {
        let at = p.with_y(MAP.floor_at(p) + 0.03);
        commands.spawn((Mesh3d(site_mesh.clone()), MeshMaterial3d(site_mat.clone()), Transform::from_translation(at)));
    }
    spawn_c4(&mut commands, &mut meshes, &mut mats);
    commands.spawn((
        Scoreboard,
        Text::new(""),
        TextLayout { justify: Justify::Center, ..default() },
        Node { position_type: PositionType::Absolute, top: px(12), width: percent(100), justify_content: JustifyContent::Center, ..default() },
    ));
    commands.spawn((
        DeathFade,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::NONE),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        // A zero-size anchor at the exact center; the lines are placed around it.
        children![(
            Node { width: px(0), height: px(0), ..default() },
            children![
                crosshair_line(Vec2::X),
                crosshair_line(Vec2::NEG_X),
                crosshair_line(Vec2::Y),
                crosshair_line(Vec2::NEG_Y),
            ],
        )],
    ));
    // CS 1.6 HUD: amber health and armor bottom-left, money and ammo bottom-right; status messages centered.
    let amber = TextColor(HUD_AMBER);
    let big = TextFont { font_size: FontSize::Px(34.0), ..default() };
    let icon = |w: f32, h: f32| (Node { width: px(w), height: px(h), ..default() }, BackgroundColor(HUD_AMBER));
    commands.spawn((
        Node { position_type: PositionType::Absolute, left: px(20), bottom: px(14), align_items: AlignItems::Center, column_gap: px(10), ..default() },
        children![
            // Health cross.
            (Node { width: px(24), height: px(24), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                children![icon(24.0, 7.0), (Node { position_type: PositionType::Absolute, width: px(7), height: px(24), ..default() }, BackgroundColor(HUD_AMBER))]),
            (HealthText, Text::new(""), big.clone(), amber, Node { width: px(80), ..default() }),
            // Armor: a shield-ish outline.
            (Node { width: px(18), height: px(22), border: UiRect::all(px(4)), border_radius: BorderRadius::bottom(px(9)), ..default() }, BorderColor::all(HUD_AMBER)),
            (ArmorText, Text::new(""), big.clone(), amber),
        ],
    ));
    commands.spawn((
        Node { position_type: PositionType::Absolute, right: px(20), bottom: px(56), ..default() },
        children![(MoneyText, Text::new(""), big.clone(), amber)],
    ));
    commands.spawn((
        Node { position_type: PositionType::Absolute, bottom: px(14), width: percent(100), justify_content: JustifyContent::Center, ..default() },
        children![(TimerText, Text::new(""), big.clone(), amber)],
    ));
    commands.spawn((
        Hud,
        Text::new(""),
        TextLayout { justify: Justify::Center, ..default() },
        Node { position_type: PositionType::Absolute, bottom: px(90), width: percent(100), justify_content: JustifyContent::Center, ..default() },
    ));
    let section = |team: Team, color: Color| (ScoreSection(team), Text::new(""), TextColor(color), TextFont { font_size: FontSize::Px(17.0), ..default() });
    commands.spawn((
        ScorePanel,
        Visibility::Hidden,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
        children![(
            Node { flex_direction: FlexDirection::Column, row_gap: px(18), padding: UiRect::all(px(20)), border_radius: BorderRadius::all(px(6)), ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.78)),
            children![section(Team::T, Color::srgb(1.0, 0.45, 0.3)), section(Team::Ct, Color::srgb(0.55, 0.7, 1.0))],
        )],
    ));
    commands.spawn((
        Node { position_type: PositionType::Absolute, top: percent(26), width: percent(100), justify_content: JustifyContent::Center, ..default() },
        children![(AnnounceText, Text::new(""), TextFont { font_size: FontSize::Px(40.0), ..default() }, TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.85) })],
    ));
    let radio_lines: String = std::iter::once("RADIO".to_string())
        .chain(Radio::MENU.iter().enumerate().map(|(i, r)| format!("{}  {}", i + 1, r.text())))
        .collect::<Vec<_>>()
        .join("\n");
    commands.spawn((
        RadioMenu,
        Text::new(radio_lines),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        TextColor(Color::srgb(1.0, 0.85, 0.35)),
        Node { position_type: PositionType::Absolute, left: px(24), top: percent(28), padding: UiRect::all(px(14)), border: UiRect::left(px(3)), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        BorderColor::all(HUD_AMBER),
        Visibility::Hidden,
    ));
    commands.spawn((
        Node { position_type: PositionType::Absolute, top: percent(58), width: percent(100), justify_content: JustifyContent::Center, ..default() },
        children![(TargetText, Text::new(""), TextFont { font_size: FontSize::Px(18.0), ..default() }, TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.85) })],
    ));
    commands.spawn((
        ChatText,
        Text::new(""),
        TextColor(Color::srgb(1.0, 0.85, 0.35)),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.85) },
        Node { position_type: PositionType::Absolute, left: px(20), bottom: px(70), ..default() },
    ));
    commands.spawn((
        NetGraphText,
        Text::new(""),
        TextFont { font_size: FontSize::Px(13.0), ..default() },
        Node { position_type: PositionType::Absolute, left: px(12), top: px(12), ..default() },
        Visibility::Hidden,
    ));
    // Scope: a round lens whose huge black outline blacks out the rest of the screen, with thin crosshair lines
    // through it, like CS 1.6's AWP scope.
    let hair = |horizontal: bool| {
        let (width, height) = if horizontal { (percent(100), px(2)) } else { (px(2), percent(100)) };
        let (left, top) = if horizontal { (percent(0), percent(50)) } else { (percent(50), percent(0)) };
        let margin = if horizontal { UiRect::top(px(-1)) } else { UiRect::left(px(-1)) };
        (Node { position_type: PositionType::Absolute, left, top, width, height, margin, ..default() }, BackgroundColor(Color::BLACK))
    };
    commands.spawn((
        Scope,
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        children![(
            Node { width: vh(96), height: vh(96), border_radius: BorderRadius::MAX, ..default() },
            Outline::new(px(4000), Val::ZERO, Color::BLACK),
            children![hair(true), hair(false)],
        )],
    ));
    commands.spawn((
        HurtFlash,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::NONE),
    ));
    commands.spawn((
        AmmoText,
        Text::new(""),
        TextFont { font_size: FontSize::Px(34.0), ..default() },
        TextColor(HUD_AMBER),
        Node { position_type: PositionType::Absolute, right: px(20), bottom: px(14), ..default() },
    ));
    // CS 1.6 buy menu: dark panel, amber title, numbered entries, unaffordable ones grayed out.
    let row = |i: usize| (BuyRow(i), Text::new(""), TextFont { font_size: FontSize::Px(18.0), ..default() });
    commands.spawn((
        BuyMenu,
        Node {
            position_type: PositionType::Absolute,
            left: px(24),
            top: percent(28),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            padding: UiRect::all(px(14)),
            border: UiRect::left(px(3)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        BorderColor::all(HUD_AMBER),
        Visibility::Hidden,
        children![
            (BuyTitle, Text::new(""), TextColor(HUD_AMBER), TextFont { font_size: FontSize::Px(20.0), ..default() }),
            row(0),
            row(1),
            row(2),
            row(3),
            row(4),
            (Text::new("B / Esc  close"), TextColor(Color::srgb(0.55, 0.55, 0.55)), TextFont { font_size: FontSize::Px(13.0), ..default() }),
        ],
    ));
    commands.spawn((
        KillFeed,
        Node { position_type: PositionType::Absolute, right: px(16), top: px(12), flex_direction: FlexDirection::Column, align_items: AlignItems::End, row_gap: px(3), ..default() },
    ));
}

/// Keys that act once per press: buy menu, weapon slots, reload, zoom.
fn controls(keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>, mut g: ResMut<Game>) {
    let can_buy = g.round.is_some_and(|r| r.buy_secs > 0.0) && g.hp > 0;
    if keys.just_pressed(KeyCode::KeyB) && can_buy {
        g.menu = !g.menu;
    }
    if !can_buy || keys.just_pressed(KeyCode::Escape) {
        g.menu = false;
    }
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5];
    if keys.just_pressed(KeyCode::KeyZ) && g.hp > 0 {
        (g.radio_menu, g.menu) = (!g.radio_menu, false);
    }
    if keys.just_pressed(KeyCode::Escape) || g.hp <= 0 {
        g.radio_menu = false;
    }
    if let Some(n) = digits.iter().position(|&k| keys.just_pressed(k)) {
        if g.radio_menu {
            (g.want_radio, g.radio_menu) = (n as u8 + 1, false);
        } else if g.menu {
            (g.want_buy, g.menu) = (n as u8 + 1, false);
        } else if n < 3 {
            g.want_slot = n as u8 + 1;
        }
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        g.want_slot = 4;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        g.want_reload = true;
    }
    if mouse.just_pressed(MouseButton::Right) && g.pred.weapon() == Weapon::Awp {
        g.zoom = !g.zoom;
    }
    if g.pred.weapon() != Weapon::Awp || g.pred.reloading > 0 {
        g.zoom = false;
    }
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn look(motion: Res<AccumulatedMouseMotion>, cursor: Single<&CursorOptions, With<PrimaryWindow>>, mut g: ResMut<Game>) {
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    let sens = if g.zoom { MOUSE_SENS * ZOOM_FOV / FOV } else { MOUSE_SENS };
    g.yaw -= motion.delta.x * sens;
    g.pitch = (g.pitch - motion.delta.y * sens).clamp(-1.55, 1.55);
}

/// Every frame rather than every tick, so snapshots aren't left waiting in the socket.
fn receive(mut net: ResMut<Net>, mut g: ResMut<Game>) {
    for packet in net.recv() {
        match decode::<ServerMsg>(&packet) {
            Some(ServerMsg::Snapshot(snap)) => on_snapshot(&mut g, snap),
            Some(ServerMsg::WrongBuild { server }) => g.wrong_build = Some(server),
            None => g.incompatible = true,
        }
    }
}

fn net_tick(
    mut net: ResMut<Net>,
    mut g: ResMut<Game>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
) {
    // Grow the buffer fast when snapshots arrive late, shrink it slowly once they don't.
    let target = (g.jitter_ticks + 2.0).clamp(g.min_interp_ticks, 0.15 * TICK_HZ);
    let rate = if target > g.interp_ticks { 0.2 } else { 0.005 };
    g.interp_ticks += (target - g.interp_ticks) * rate;


    let axis = |pos, neg| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
    let active = cursor.grab_mode != CursorGrabMode::None;
    g.seq += 1;
    let inp = Input {
        seq: g.seq,
        forward: axis(KeyCode::KeyW, KeyCode::KeyS),
        right: axis(KeyCode::KeyD, KeyCode::KeyA),
        yaw: g.yaw,
        pitch: g.pitch,
        jump: keys.any_pressed([KeyCode::Space, KeyCode::Backspace]),
        walk: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        use_key: keys.pressed(KeyCode::KeyE),
        // FPS16_FIRE clicks repeatedly, for screenshots of effects.
        fire: (active && !g.menu && mouse.pressed(MouseButton::Left)) || (std::env::var_os("FPS16_FIRE").is_some() && g.seq % 16 < 8),
        slot: std::mem::take(&mut g.want_slot),
        reload: std::mem::take(&mut g.want_reload),
        zoom: g.zoom,
        buy: std::mem::take(&mut g.want_buy),
        radio: std::mem::take(&mut g.want_radio),
        view_tick: (server_tick(&g) - g.interp_ticks) as f32,
    };

    g.prev_pos = g.pred.pos;
    if g.me.is_some() && g.hp > 0 {
        let mut pred = g.pred;
        let was_reloading = pred.reloading > 0;
        if let Some((weapon, d)) = step_player(&mut pred, &inp) {
            let o = eye(&pred);
            g.sounds.push((Sound::Gun(weapon), 0.6));
            g.kick = 1.0;
            let (t, normal) = trace_world_normal(o, d);
            add_tracer(&mut g, weapon, muzzle(o, d, weapon), o + d * t);
            if weapon != Weapon::Knife {
                g.fx.push(FxKind::Muzzle(muzzle(o, d, weapon)));
            }
            if t < weapon.spec().range.min(RANGE - 1.0) {
                g.fx.push(FxKind::Impact(o + d * t, normal));
                g.sounds.push((Sound::Impact, 0.25));
            }
            if weapon == Weapon::Awp {
                g.zoom = false;
            }
        }
        if pred.reloading > 0 && !was_reloading {
            g.sounds.push((Sound::Reload, 0.5));
        }
        // Footsteps when running; walking (Shift) is silent.
        let speed = pred.vel.with_y(0.0).length();
        if pred.on_ground && speed > 6.35 * WALK_SPEED + 0.3 {
            g.stride += speed * DT;
            if g.stride > 1.8 {
                g.stride = 0.0;
                g.steps_played += 1;
                let v = g.steps_played;
                g.sounds.push((Sound::Step(v), 0.3));
            }
        }
        g.pred = pred;
    }
    g.pending.push_back(inp);
    while g.pending.len() > 128 {
        g.pending.pop_front();
    }
    if g.seq % INPUT_EVERY == 0 {
        let start = g.pending.len().saturating_sub(REDUNDANCY);
        let inputs = g.pending.iter().skip(start).copied().collect();
        let ping = g.ping_ms.round() as u16;
        let packet = InputPacket { build: BUILD.into(), name: player_name(), ping, time: clock(), inputs };
        net.send(encode(&packet));
    }
}

fn on_snapshot(g: &mut Game, snap: Snapshot) {
    if snap.tick <= g.latest_tick {
        return;
    }
    g.latest_tick = snap.tick;
    let sample = snap.tick as f64 - clock() * TICK_HZ;
    g.tick_offset = Some(match g.tick_offset {
        Some(o) if (sample - o).abs() < 8.0 => o + (sample - o) * 0.05,
        _ => sample,
    });
    g.me = Some(snap.you);
    g.players = snap.players.clone();
    for p in &snap.players {
        g.teams.insert(p.id, p.team);
        if g.names.get(&p.id) != Some(&p.name) {
            g.names.insert(p.id, p.name.clone());
        }
    }
    for r in &snap.radio {
        g.sounds.push((Sound::Radio(r.call), 0.75));
        let line = format!("{} (RADIO): {}", name(g, r.from), r.call.text());
        g.chat.push_back((line, Instant::now()));
        if g.chat.len() > 4 {
            g.chat.pop_front();
        }
    }
    for h in &snap.hits {
        if Some(h.attacker) == g.me {
            g.sounds.push((if h.headshot { Sound::Headshot } else { Sound::HitBody }, 0.7));
            if h.killed {
                g.sounds.push((Sound::Kill, 0.6));
                g.sounds.push((Sound::Voice(Announce::EnemyDown), 0.6));
            }
        }
        if Some(h.victim) == g.me {
            g.sounds.push((Sound::Hurt, 0.8));
            g.hurt_at = Some(Instant::now());
        } else if let Some(v) = snap.players.iter().find(|p| p.id == h.victim) {
            let at = v.state.pos + Vec3::Y * if h.headshot { 1.65 } else { 1.1 };
            g.fx.push(FxKind::Blood(at));
        }
        if h.killed {
            let side = |id: u32| (name(g, id), g.teams.get(&id).copied());
            let line = KillLine { killer: side(h.attacker), weapon: h.weapon, victim: side(h.victim), headshot: h.headshot, at: Instant::now() };
            g.kill_feed.push_back(line);
            if g.kill_feed.len() > 5 {
                g.kill_feed.pop_front();
            }
            g.feed_version += 1;
        }
    }
    let prev = g.round.replace(snap.round);
    if let Some(prev) = prev {
        let r = snap.round;
        let mut say = |a: Announce, text: &str| {
            g.sounds.push((Sound::Voice(a), 0.9));
            g.announce = Some((text.into(), Instant::now()));
        };
        // Keyed on entering a phase, not on the exact previous one: a round can end before a live snapshot arrives.
        match (prev.phase, r.phase) {
            (Phase::Freeze, Phase::Live) => say(Announce::Go, ""),
            (Phase::End { .. }, _) => {}
            (_, Phase::End { winner: Team::T }) => say(Announce::TWin, "Terrorists Win!"),
            (_, Phase::End { winner: Team::Ct }) => say(Announce::CtWin, "Counter-Terrorists Win!"),
            _ => {}
        }
        match (prev.bomb, r.bomb) {
            (Bomb::Carried(_), Bomb::Planted(_)) => say(Announce::Planted, "The bomb has been planted."),
            (Bomb::Planted(_), Bomb::Defused(_)) => say(Announce::Defused, "Bomb has been defused."),
            _ => {}
        }
    }
    if prev.map(|r| r.number) != Some(snap.round.number) {
        // New round: everyone is back in spawn, so face the way the server spawned us.
        if let Some(me) = snap.players.iter().find(|p| p.id == snap.you) {
            (g.yaw, g.pitch) = (me.state.yaw, 0.0);
        }
        g.died_at = None;
        g.explosion = None;
    }
    if let Bomb::Exploded(pos) = snap.round.bomb {
        if !matches!(prev.map(|r| r.bomb), Some(Bomb::Exploded(_))) {
            g.explosion = Some((pos, clock()));
            let volume = 1.0 / (1.0 + pos.distance(g.pred.pos) / 30.0);
            g.sounds.push((Sound::Explosion, volume));
        }
    }
    if let Some(last) = g.net_graph.last_snap.replace(Instant::now()) {
        let gap = last.elapsed().as_secs_f64();
        g.net_graph.max_gap_ms = g.net_graph.max_gap_ms.max(gap as f32 * 1000.0);
        // Decays by elapsed time (half-life 2 s) so one hitch, e.g. while the window opens, doesn't linger.
        g.jitter_ticks = (gap * TICK_HZ).max(g.jitter_ticks * 0.5f64.powf(gap / 2.0));
    }

    if snap.echo > 0.0 {
        let rtt_ms = ((clock() - snap.echo) as f32 - snap.echo_held).max(0.0) * 1000.0;
        g.ping_ms = g.ping_ms * 0.9 + rtt_ms * 0.1;
    }

    if let Some(me) = snap.players.iter().find(|p| p.id == snap.you) {
        if me.money < g.money && g.round.is_some_and(|r| r.buy_secs > 0.0) {
            g.sounds.push((Sound::Buy, 0.6));
        }
        (g.armor, g.money) = (me.armor, me.money);
        if g.hp > 0 && me.hp <= 0 {
            g.sounds.push((Sound::Death, 0.8));
            g.died_at = Some(Instant::now());
        }
        if me.hp > 0 {
            g.died_at = None;
        }
        g.hp = me.hp;
        g.kills = me.kills;
        g.deaths = me.deaths;
        while g.pending.front().is_some_and(|i| i.seq <= snap.ack) {
            g.pending.pop_front();
        }
        let mut s = me.state;
        if me.hp > 0 {
            for inp in &g.pending {
                step_player(&mut s, inp);
            }
        }
        let err = g.pred.pos - s.pos;
        g.net_graph.max_fix_cm = g.net_graph.max_fix_cm.max(err.length() * 100.0);
        if err.length() < 2.0 {
            g.smooth += err;
        } else {
            g.smooth = Vec3::ZERO;
        }
        g.prev_pos -= err;
        g.pred = s;
    }

    for s in snap.shots.iter().filter(|s| s.shooter != snap.you) {
        let d = (s.to - s.from).normalize_or_zero();
        add_tracer(g, s.weapon, muzzle(s.from, d, s.weapon), s.to);
        if s.weapon != Weapon::Knife {
            g.fx.push(FxKind::Muzzle(muzzle(s.from, d, s.weapon)));
            // A wall or floor where the shot ended (not a player).
            let (t, normal) = trace_world_normal(s.from, d);
            if (t - s.from.distance(s.to)).abs() < 0.05 {
                g.fx.push(FxKind::Impact(s.to, normal));
            }
        }
        let volume = 0.6 / (1.0 + s.from.distance(g.pred.pos) / 12.0);
        g.sounds.push((Sound::Gun(s.weapon), volume));
    }
    g.snapshots.push_back(snap);
    while g.snapshots.len() > 64 {
        g.snapshots.pop_front();
    }
}

fn render_self(
    cam: Single<(&mut Transform, &mut Projection), With<Camera3d>>,
    mut g: ResMut<Game>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
) {
    let (mut cam, mut projection) = cam.into_inner();
    if let Projection::Perspective(p) = &mut *projection {
        p.fov = if g.zoom { ZOOM_FOV } else { FOV };
    }
    g.smooth *= (-time.delta_secs() * 15.0).exp();
    if let Some((_, eye, yaw, pitch)) = g.spectate.filter(|_| spectating(&g)) {
        cam.translation = eye;
        cam.rotation = look_rot(yaw, pitch);
        return;
    }
    let pos = g.prev_pos.lerp(g.pred.pos, fixed.overstep_fraction()) + g.smooth;
    cam.translation = pos + Vec3::Y * EYE;
    let (side, up) = punch(&g.pred);
    cam.rotation = look_rot(g.yaw + side, g.pitch + up);
}

fn render_remotes(
    mut commands: Commands,
    mut q: Query<(Entity, &Remote, &mut Rig, &mut Transform, &mut Visibility)>,
    mut parts: Query<(&mut Transform, &mut Visibility), Without<Remote>>,
    time: Res<Time>,
    mut g: ResMut<Game>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let render_tick = server_tick(&g) - g.interp_ticks;
    if g.snapshots.back().is_some_and(|s| render_tick > s.tick as f64) {
        g.net_graph.starved += 1;
    }
    let Some(newest) = g.snapshots.back() else { return };
    let (a, b) = g
        .snapshots
        .iter()
        .zip(g.snapshots.iter().skip(1))
        .find(|(a, b)| a.tick as f64 <= render_tick && render_tick <= b.tick as f64)
        .unwrap_or((newest, newest));
    let f = if b.tick > a.tick { ((render_tick - a.tick as f64) / (b.tick - a.tick) as f64) as f32 } else { 0.0 };
    // Starved: keep players moving on their last velocity for a moment rather than freezing them.
    let extrapolate = ((render_tick - newest.tick as f64) / TICK_HZ).clamp(0.0, 0.1) as f32;

    let my_team = g.me.and_then(|me| g.teams.get(&me)).copied();
    let carrier = match newest.round.bomb {
        Bomb::Carried(id) => Some(id),
        _ => None,
    };
    let (mut spectate, mut carrier_pos) = (None, None);
    let mut seen = HashMap::new();
    for p in b.players.iter().filter(|p| Some(p.id) != g.me) {
        let from = a.players.iter().find(|q| q.id == p.id).map_or(p.state, |q| q.state);
        let pos = from.pos.lerp(p.state.pos, f) + p.state.vel * extrapolate;
        let yaw = from.yaw + wrap_angle(p.state.yaw - from.yaw) * f;
        let pitch = from.pitch + (p.state.pitch - from.pitch) * f;
        // Dead as soon as we hear of it (the body falls and stays); hidden while a respawn is being interpolated
        // past, so the teleport is never drawn as a slide across the map.
        let alive_in = |s: &Snapshot| s.players.iter().find(|q| q.id == p.id).is_some_and(|q| q.hp > 0);
        let alive = alive_in(newest) && alive_in(a) && p.hp > 0;
        let life = if alive { Life::Alive } else if !alive_in(newest) { Life::Dead } else { Life::Respawning };
        if alive && spectate.is_none() && Some(p.team) == my_team {
            spectate = Some((p.id, pos + Vec3::Y * EYE, yaw, pitch));
        }
        if Some(p.id) == carrier {
            carrier_pos = Some(pos);
        }
        seen.insert(p.id, (pos, yaw, pitch, life, p.state.weapon()));
    }
    (g.spectate, g.carrier_pos) = (spectate, carrier_pos);
    // Whoever is under the crosshair, walls permitting, for the "Friend: / Enemy:" line.
    let (eye, dir) = (g.pred.pos + Vec3::Y * EYE, look_rot(g.yaw, g.pitch) * Vec3::NEG_Z);
    let wall = trace_world(eye, dir);
    g.aim_target = seen
        .iter()
        .filter(|(_, v)| v.3 == Life::Alive)
        .filter_map(|(&id, v)| trace_capsule(eye, dir, v.0).filter(|&t| t < wall).map(|t| (id, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id);
    // We're looking out of their eyes, so their own model would only block the view.
    let watched = spectate.filter(|_| spectating(&g)).map(|s| s.0);

    for (e, r, mut rig, mut t, mut vis) in &mut q {
        let Some((pos, yaw, pitch, life, weapon)) = seen.remove(&r.0) else {
            commands.entity(e).despawn();
            continue;
        };
        let alive = life == Life::Alive;
        // A dead player topples over, forwards or backwards, and stays down until the next round.
        let fall = match life {
            Life::Dead => {
                let since = *rig.died_at.get_or_insert(clock());
                ((clock() - since) as f32 / 0.45).min(1.0).powi(2)
            }
            _ => {
                rig.died_at = None;
                0.0
            }
        };
        let side = if r.0 % 2 == 0 { 1.0 } else { -1.0 };
        t.translation = pos + Vec3::Y * 0.13 * fall;
        t.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(side * 1.5 * fall);
        // Players pass through each other; hide anyone overlapping our camera rather than fill the screen with them.
        // ponytail: no player-player collision. Add it to the shared movement if walking through people matters.
        let overlapping = pos.with_y(0.0).distance(g.pred.pos.with_y(0.0)) < RADIUS * 2.0;
        let shown = match life {
            Life::Alive => watched != Some(r.0) && !overlapping,
            Life::Dead => true,
            Life::Respawning => false,
        };
        *vis = if shown { Visibility::Inherited } else { Visibility::Hidden };
        let (swing, moved) = rig.animate(pos, time.delta_secs());
        let (swing, pitch) = if alive { (swing, pitch) } else { (0.0, -0.4) };
        // Their footsteps, louder up close: how you hear someone coming.
        let speed = moved / time.delta_secs().max(1e-3);
        if alive && speed > 6.35 * WALK_SPEED + 0.3 && speed < 12.0 {
            rig.stride += moved;
            if rig.stride > 1.8 {
                rig.stride = 0.0;
                let dist = pos.distance(g.pred.pos);
                if dist < 35.0 {
                    let v = r.0 + (rig.phase * 10.0) as u32;
                    g.sounds.push((Sound::Step(v), 0.45 / (1.0 + dist / 5.0)));
                }
            }
        }
        for (leg, dir) in rig.legs.into_iter().zip([1.0, -1.0]) {
            if let Ok((mut leg, _)) = parts.get_mut(leg) {
                leg.rotation = Quat::from_rotation_x(swing * dir);
            }
        }
        if let Ok((mut arms, _)) = parts.get_mut(rig.arms) {
            arms.rotation = Quat::from_rotation_x(pitch);
        }
        for (i, &gun) in rig.guns.iter().enumerate() {
            if let Ok((_, mut v)) = parts.get_mut(gun) {
                *v = if i == weapon as usize { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
    }
    for (id, (pos, yaw, ..)) in seen {
        let team = g.teams.get(&id).copied().unwrap_or(Team::T);
        let root = (Remote(id), Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)));
        spawn_soldier(&mut commands, &mut meshes, &mut mats, team, root);
    }
}

fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[allow(clippy::type_complexity)]
fn hud(
    mut texts: ParamSet<(
        Single<&mut Text, With<Hud>>,
        Single<&mut Text, With<HealthText>>,
        Single<&mut Text, With<ArmorText>>,
        Single<&mut Text, With<MoneyText>>,
        Single<(&mut Text, &mut Visibility), With<NetGraphText>>,
    )>,
    keys: Res<ButtonInput<KeyCode>>,
    mut g: ResMut<Game>,
    time: Res<Time>,
) {
    let ping = g.ping_ms;
    let interp = g.interp_ticks / TICK_HZ * 1000.0;
    let n = &mut g.net_graph;
    n.frames += 1;
    n.worst_frame_ms = n.worst_frame_ms.max(time.delta_secs() * 1000.0);
    if n.since.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
        n.line = format!(
            "fps {} (worst frame {:.0} ms)   ping {ping:.0} ms   worst snapshot gap {:.0} ms   correction {:.0} cm   interp {interp:.0} ms, starved {} frames",
            n.frames, n.worst_frame_ms, n.max_gap_ms, n.max_fix_cm, n.starved
        );
        if std::env::var_os("FPS16_NETLOG").is_some() {
            println!("{}", n.line);
        }
        (n.max_gap_ms, n.max_fix_cm, n.starved, n.since) = (0.0, 0.0, 0, Some(Instant::now()));
        (n.frames, n.worst_frame_ms) = (0, 0.0);
    }
    let status = match (g.incompatible, g.me) {
        _ if g.me.is_none() && g.wrong_build.is_some() => format!(
            "SERVER IS ON BUILD {}, YOU HAVE BUILD {BUILD} - restart the game to update",
            g.wrong_build.as_deref().unwrap_or("?")
        ),
        (true, None) => "SERVER IS A DIFFERENT VERSION - update both".into(),
        (_, None) => "connecting...".into(),
        _ if g.hp > 0 => String::new(),
        _ => "You are dead - spectating".into(),
    };
    texts.p0().0 = status;
    let alive = g.hp > 0 && g.me.is_some();
    texts.p1().0 = if alive { g.hp.to_string() } else { String::new() };
    texts.p2().0 = if alive { g.armor.to_string() } else { String::new() };
    texts.p3().0 = format!("$ {}", g.money);
    // Network stats, like CS's net_graph: toggle with N.
    let mut net = texts.p4();
    let (text, vis) = &mut *net;
    if keys.just_pressed(KeyCode::KeyN) {
        **vis = if **vis == Visibility::Hidden { Visibility::Inherited } else { Visibility::Hidden };
    }
    text.0 = format!("{}\nK {}  D {}", g.net_graph.line, g.kills, g.deaths);
}

fn draw_tracers(mut gizmos: Gizmos, mut g: ResMut<Game>, time: Res<Time>) {
    for (a, b, ttl, life, color) in &mut g.tracers {
        gizmos.line(*a, *b, color.with_alpha(*ttl / *life));
        *ttl -= time.delta_secs();
    }
    g.tracers.retain(|t| t.2 > 0.0);
}

/// Barrel tip for a shot from eye `o` along `d`, matching the first-person model, so tracers leave the gun.
fn muzzle(o: Vec3, d: Vec3, weapon: Weapon) -> Vec3 {
    let right = d.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(d);
    let at = viewmodel_offset(weapon);
    o + right * at.x + up * at.y + d * (barrel_length(weapon) - at.z)
}

/// Dead and the fade to black is done: watch a teammate instead.
fn spectating(g: &Game) -> bool {
    g.me.is_some() && g.hp <= 0 && g.died_at.is_none_or(|t| t.elapsed().as_secs_f32() > 1.6)
}

fn render_bomb(
    mut bomb: Single<(&mut Transform, &mut Visibility), With<BombModel>>,
    led: Single<(&BombLed, &mut PointLight)>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    g: Res<Game>,
    mut gizmos: Gizmos,
) {
    // The light flashes with each beep while planted.
    let planted = matches!(g.round.map(|r| r.bomb), Some(Bomb::Planted(_)));
    let lit = planted && clock() - g.last_beep < 0.09;
    let (led, mut light) = led.into_inner();
    if let Some(mut m) = mats.get_mut(&led.0) {
        m.emissive = if lit { LinearRgba::rgb(40.0, 0.0, 0.0) } else { LinearRgba::rgb(0.3, 0.0, 0.0) };
    }
    light.intensity = if lit { 4000.0 } else { 0.0 };
    let (t, vis) = &mut *bomb;
    let at = match g.round.map(|r| r.bomb) {
        Some(Bomb::Dropped(p) | Bomb::Planted(p)) => Some(p + Vec3::Y * 0.045),
        // On the carrier's back.
        Some(Bomb::Carried(_)) => g.carrier_pos.map(|p| p + Vec3::Y * 1.1),
        _ => None,
    };
    **vis = if at.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    t.translation = at.unwrap_or_default();
    if let Some((pos, start)) = g.explosion {
        let age = (clock() - start) as f32;
        if age < 1.0 {
            let color = Color::srgba(1.0, 0.6, 0.1, 1.0 - age);
            gizmos.sphere(Isometry3d::from_translation(pos), BOMB_RADIUS * age.sqrt(), color);
        }
    }
}

/// Beeps speed up as the timer runs down, like the real C4.
fn bomb_beep(mut g: ResMut<Game>) {
    let Some(Round { bomb: Bomb::Planted(pos), time_left, .. }) = g.round else { return };
    if clock() < g.next_beep {
        return;
    }
    g.last_beep = clock();
    g.next_beep = clock() + 0.12 + 0.88 * (time_left / BOMB_SECS) as f64;
    let volume = 0.7 / (1.0 + pos.distance(g.pred.pos) / 15.0);
    g.sounds.push((Sound::Beep, volume));
}

fn scoreboard(
    mut text: Single<&mut Text, (With<Scoreboard>, Without<TimerText>)>,
    mut timer: Single<&mut Text, With<TimerText>>,
    g: Res<Game>,
) {
    let Some(r) = g.round else { return };
    let secs = r.time_left.ceil() as u32;
    let my_team = g.me.and_then(|me| g.teams.get(&me)).copied();
    let message = match (r.phase, r.bomb, r.progress) {
        (Phase::End { winner }, Bomb::Exploded(_), _) => format!("{} win - the bomb exploded", winner.name()),
        (Phase::End { winner }, Bomb::Defused(_), _) => format!("{} win - the bomb was defused", winner.name()),
        (Phase::End { winner }, ..) => format!("{} win", winner.name()),
        (Phase::Freeze, ..) => format!("Round {} - get ready", r.number),
        (_, _, Some((id, p))) if Some(id) == g.me => {
            let verb = if matches!(r.bomb, Bomb::Planted(_)) { "Defusing" } else { "Planting" };
            let bar = "#".repeat((p * 20.0) as usize) + &".".repeat(20 - (p * 20.0) as usize);
            format!("{verb} [{bar}]")
        }
        (_, Bomb::Planted(_), Some(_)) => "BOMB PLANTED - being defused!".into(),
        (_, Bomb::Planted(_), None) => "BOMB PLANTED".into(),
        (_, Bomb::Carried(id), _) if Some(id) == g.me => "You have the bomb - hold E on A or B to plant".into(),
        (_, Bomb::Dropped(_), _) if my_team == Some(Team::T) => "The bomb was dropped - pick it up".into(),
        _ => String::new(),
    };
    let side = match my_team {
        Some(t) => t.name(),
        None => "",
    };
    timer.0 = format!("{}:{:02}", secs / 60, secs % 60);
    text.0 = format!("T {}  -  {} CT   ({side})\n{message}", r.score[0], r.score[1]);
}

fn play_sounds(mut commands: Commands, mut g: ResMut<Game>, sounds: Res<Sounds>) {
    for (sound, volume) in g.sounds.drain(..) {
        let handle = match sound {
            Sound::Gun(w) => &sounds.guns[&w],
            Sound::Reload => &sounds.reload,
            Sound::Buy => &sounds.buy,
            Sound::Headshot => &sounds.headshot,
            Sound::HitBody => &sounds.hit_body,
            Sound::Kill => &sounds.kill,
            Sound::Hurt => &sounds.hurt,
            Sound::Death => &sounds.death,
            Sound::Beep => &sounds.beep,
            Sound::Explosion => &sounds.explosion,
            Sound::Step(v) => &sounds.steps[v as usize % 4],
            Sound::Impact => &sounds.impact,
            Sound::Voice(a) => &sounds.voices.iter().find(|(v, _)| *v == a).unwrap().1,
            Sound::Radio(r) => &sounds.radio.iter().find(|(v, _)| *v == r).unwrap().1,
        };
        commands.spawn((
            AudioPlayer::new(handle.clone()),
            PlaybackSettings { volume: Volume::Linear(volume), ..PlaybackSettings::DESPAWN },
        ));
    }
}

fn death_fade(mut fade: Single<&mut BackgroundColor, With<DeathFade>>, g: Res<Game>) {
    // Fade to black, hold, then fade back in on a teammate's view.
    let alpha = g.died_at.map_or(0.0, |t| match t.elapsed().as_secs_f32() {
        t if t < 0.8 => t / 0.8,
        t if t < 1.6 => 1.0,
        t => (1.0 - (t - 1.6) / 0.8).max(0.0),
    });
    fade.0 = Color::BLACK.with_alpha(alpha);
}

fn crosshair_line(dir: Vec2) -> impl Bundle {
    let (w, h) = if dir.x != 0.0 { (px(9), px(2)) } else { (px(2), px(9)) };
    (Crosshair(dir), Node { position_type: PositionType::Absolute, width: w, height: h, ..default() }, BackgroundColor(CROSSHAIR_GREEN))
}

/// CS-style dynamic crosshair: the four lines spread with movement, jumping and recoil, like the real spread.
/// Grayed out while reloading or drawing, so a held trigger firing late doesn't read as lag. Hidden when scoped.
fn crosshair(mut q: Query<(&Crosshair, &mut Node, &mut BackgroundColor, &mut Visibility)>, g: Res<Game>) {
    let busy = g.pred.reloading > 0 || g.pred.fire_cd as f32 > g.pred.weapon().spec().cycle * TICK_HZ as f32;
    let speed = (g.pred.vel.with_y(0.0).length() / 6.35).min(1.0);
    let air = if g.pred.on_ground { 0.0 } else { 1.0 };
    let gap = 4.0 + 18.0 * speed + 14.0 * air + 3.0 * g.pred.recoil.min(6.0);
    for (line, mut node, mut color, mut vis) in &mut q {
        // Lines are 9 px long and 2 px thick; offset each from the center by the gap.
        let at = line.0 * (gap + 4.5);
        let (w, h) = if line.0.x != 0.0 { (9.0, 2.0) } else { (2.0, 9.0) };
        node.left = px(at.x - w / 2.0);
        node.top = px(-at.y - h / 2.0);
        color.0 = if busy { Color::srgba(0.5, 0.5, 0.5, 0.6) } else { CROSSHAIR_GREEN };
        *vis = if g.zoom { Visibility::Hidden } else { Visibility::Inherited };
    }
}

fn add_tracer(g: &mut Game, weapon: Weapon, from: Vec3, to: Vec3) {
    let life = match weapon {
        Weapon::Knife => return,
        Weapon::Awp => 0.35,
        _ => 0.06,
    };
    g.tracers.push((from, to, life, life, Color::srgba(1.0, 0.9, 0.5, 0.8)));
}

fn name(g: &Game, id: u32) -> String {
    g.names.get(&id).cloned().unwrap_or_else(|| format!("player {id}"))
}

/// The logged-in macOS user (also set for apps launched from Finder).
fn player_name() -> String {
    static NAME: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "player".into()));
    NAME.clone()
}

/// First-person weapon animation state that lives only on this client.
#[derive(Default)]
struct WeaponAnim {
    held: Option<Weapon>,
    drawn_at: f64,
    bob: f32,
    sway: Vec2,
    last_look: Vec2,
}

/// Shows the held weapon with CS-style motion: a figure-eight bob while running, sway behind mouse turns, a kick
/// per shot, raising on draw, a dip while reloading and a slash for the knife. Hidden when dead or scoped.
fn viewmodel(
    mut q: Query<(&Viewmodel, &mut Transform, &mut Visibility)>,
    mut g: ResMut<Game>,
    mut anim: Local<WeaponAnim>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    let held = g.pred.weapon();
    if anim.held != Some(held) {
        (anim.held, anim.drawn_at) = (Some(held), clock());
    }
    let draw = (1.0 - ((clock() - anim.drawn_at) as f32 / DRAW_SECS).min(1.0)).powi(2);
    let reload = g.pred.reloading as f32 / (held.spec().reload * TICK_HZ as f32).max(1.0);
    let dip = (reload * std::f32::consts::PI).sin();

    let speed = if g.pred.on_ground { (g.pred.vel.with_y(0.0).length() / 6.35).min(1.0) } else { 0.0 };
    anim.bob += dt * 9.0 * speed;
    let bob = Vec3::new(anim.bob.sin() * 0.012, -(anim.bob * 2.0).sin().abs() * 0.01, 0.0) * speed;

    let look = Vec2::new(g.yaw, g.pitch);
    let turn = (look - anim.last_look) / dt.max(1e-3);
    anim.last_look = look;
    let target = (Vec2::new(-turn.x, turn.y) * 0.004).clamp(Vec2::splat(-0.04), Vec2::splat(0.04));
    anim.sway = anim.sway.lerp(target, (dt * 10.0).min(1.0));

    let heavy = matches!(held, Weapon::Awp | Weapon::Deagle);
    let kick = g.kick * if heavy { 1.6 } else { 1.0 };
    g.kick *= (-dt * 16.0).exp();
    let slash = if held == Weapon::Knife { g.kick } else { 0.0 };

    let show = g.hp > 0 && g.me.is_some() && !g.zoom;
    for (vm, mut t, mut vis) in &mut q {
        *vis = if show && vm.0 == held { Visibility::Inherited } else { Visibility::Hidden };
        let offset = Vec3::new(anim.sway.x, anim.sway.y - 0.15 * dip - 0.3 * draw, 0.05 * kick) + bob;
        t.translation = viewmodel_offset(vm.0) + offset;
        t.rotation = Quat::from_rotation_x(-0.6 * dip - 0.9 * draw + 0.12 * kick) * Quat::from_rotation_y(1.2 * slash);
    }
}

fn ammo_hud(mut text: Single<&mut Text, With<AmmoText>>, g: Res<Game>) {
    text.0 = match g.pred.ammo() {
        _ if g.hp <= 0 => String::new(),
        None => String::new(),
        Some((mag, reserve)) => format!("{mag}  |  {reserve}"),
    };
}

#[allow(clippy::type_complexity)]
fn buy_menu_hud(
    mut panel: Single<&mut Visibility, With<BuyMenu>>,
    mut title: Single<&mut Text, (With<BuyTitle>, Without<BuyRow>)>,
    mut rows: Query<(&BuyRow, &mut Text, &mut TextColor), Without<BuyTitle>>,
    g: Res<Game>,
) {
    let show = g.menu || std::env::var_os("FPS16_BUYMENU").is_some();
    **panel = if show { Visibility::Inherited } else { Visibility::Hidden };
    let (Some(r), Some(team)) = (g.round, g.me.and_then(|me| g.teams.get(&me))) else { return };
    title.0 = format!("BUY   ${}   {:.0}s", g.money, r.buy_secs.ceil());
    let menu = buy_menu(*team);
    for (row, mut text, mut color) in &mut rows {
        let item = menu[row.0];
        text.0 = format!("{}  {:<17}${}", row.0 + 1, item.name(), item.price());
        color.0 = if item.price() <= g.money { Color::WHITE } else { Color::srgb(0.5, 0.5, 0.5) };
    }
}

fn team_text_color(team: Option<Team>) -> Color {
    match team {
        Some(Team::T) => Color::srgb(1.0, 0.45, 0.3),
        Some(Team::Ct) => Color::srgb(0.55, 0.7, 1.0),
        None => Color::WHITE,
    }
}

/// CS 1.6 kill feed: one dark strip per kill, names in team colors, the weapon between, headshots marked.
fn kill_feed(mut commands: Commands, feed: Single<Entity, With<KillFeed>>, mut g: ResMut<Game>, mut shown: Local<u32>) {
    let before = g.kill_feed.len();
    g.kill_feed.retain(|l| l.at.elapsed().as_secs_f32() < 6.0);
    if g.kill_feed.len() != before {
        g.feed_version += 1;
    }
    if *shown == g.feed_version {
        return;
    }
    *shown = g.feed_version;
    commands.entity(*feed).despawn_children();
    let font = TextFont { font_size: FontSize::Px(16.0), ..default() };
    for line in &g.kill_feed {
        let weapon = format!("  [{}]{}  ", line.weapon.spec().name, if line.headshot { " HS" } else { "" });
        let parts = [
            (line.killer.0.clone(), team_text_color(line.killer.1)),
            (weapon, HUD_AMBER),
            (line.victim.0.clone(), team_text_color(line.victim.1)),
        ];
        let row = commands
            .spawn((Node { padding: UiRect::axes(px(8), px(3)), ..default() }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55))))
            .id();
        for (text, color) in parts {
            let t = commands.spawn((Text::new(text), TextColor(color), font.clone())).id();
            commands.entity(row).add_child(t);
        }
        commands.entity(*feed).add_child(row);
    }
}

fn hurt_flash(mut flash: Single<&mut BackgroundColor, With<HurtFlash>>, g: Res<Game>) {
    let alpha = g.hurt_at.map_or(0.0, |t| (0.35 * (1.0 - t.elapsed().as_secs_f32() / 0.4)).max(0.0));
    flash.0 = Color::srgba(0.8, 0.0, 0.0, alpha);
}

/// Monotonic seconds since the client started.
fn clock() -> f64 {
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    START.elapsed().as_secs_f64()
}

/// Estimated current server tick (fractional), as of the latest snapshot's arrival.
fn server_tick(g: &Game) -> f64 {
    g.tick_offset.map_or(0.0, |o| clock() * TICK_HZ + o)
}

fn scope(mut vis: Single<&mut Visibility, With<Scope>>, g: Res<Game>) {
    **vis = if g.zoom && g.hp > 0 { Visibility::Inherited } else { Visibility::Hidden };
}

/// The floor as one mesh following the height field, sand colored and darker the lower it goes.
fn terrain_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let (w, d) = MAP.size();
    let (mut pos, mut normals, mut colors, mut uvs, mut idx) = (vec![], vec![], vec![], vec![], vec![]);
    for z in 0..MAP.depth() {
        for x in 0..MAP.width() {
            let corner = |cx: usize, cz: usize| {
                Vec3::new(cx as f32 * CELL - w / 2.0, MAP.corner(cx, cz), cz as f32 * CELL - d / 2.0)
            };
            let quad = [corner(x, z), corner(x + 1, z), corner(x + 1, z + 1), corner(x, z + 1)];
            let normal = (quad[2] - quad[0]).cross(quad[1] - quad[3]).normalize_or(Vec3::Y);
            let normal = if normal.y < 0.0 { -normal } else { normal };
            let base = pos.len() as u32;
            for (i, p) in quad.into_iter().enumerate() {
                // Baked contact shadow: floor corners next to walls and crates are darker, like a lightmap.
                let (cx, cz) = [(x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1)][i];
                let around = [(cx.wrapping_sub(1), cz.wrapping_sub(1)), (cx, cz.wrapping_sub(1)), (cx.wrapping_sub(1), cz), (cx, cz)];
                let solid = around.iter().filter(|&&(ax, az)| !MAP.walkable(ax, az)).count() as f32;
                let shade = (0.55 + 0.45 * (p.y / 2.0).clamp(0.0, 1.0)) * (1.0 - 0.14 * solid);
                pos.push(p.to_array());
                normals.push(normal.to_array());
                // The texture carries the sand color; vertex color only darkens the low tunnels.
                colors.push([shade, shade * 0.97, shade * 0.94, 1.0]);
                uvs.push([p.x / 4.0, p.z / 4.0]);
            }
            idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(idx))
}

/// Map geometry with textures: terrain, sandstone walls with a frieze along the top, crates, doors and shutters,
/// and mountains on the horizon.
fn spawn_world(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    use crate::textures;
    let mut textured = |image: Image, alpha_mask: bool| {
        mats.add(StandardMaterial {
            base_color_texture: Some(images.add(image)),
            perceptual_roughness: 0.95,
            alpha_mode: if alpha_mask { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque },
            ..default()
        })
    };
    let (stone, frieze, ground) = (textured(textures::sandstone(), false), textured(textures::frieze(), false), textured(textures::ground(), false));
    let (wood, ammo, door) = (textured(textures::wood_crate(), false), textured(textures::ammo_crate(), false), textured(textures::door(), true));
    // A few shades of the same stone so neighboring buildings don't look cut from one block.
    let stones: Vec<Handle<StandardMaterial>> = [Color::WHITE, Color::srgb(0.93, 0.88, 0.82), Color::srgb(1.0, 0.94, 0.86)]
        .into_iter()
        .map(|tint| mats.add(StandardMaterial { base_color: tint, ..mats.get(&stone).unwrap().clone() }))
        .collect();

    commands.spawn((Mesh3d(meshes.add(terrain_mesh())), MeshMaterial3d(ground.clone())));
    // Desert out to the mountains, just below the lowest floor so it never shows inside the map.
    let desert = mats.add(StandardMaterial {
        uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(300.0)),
        ..mats.get(&ground).unwrap().clone()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(1200.0, 1200.0))),
        MeshMaterial3d(desert),
        Transform::from_xyz(0.0, -0.5, 0.0),
        bevy::light::NotShadowCaster,
    ));
    for &(c, h) in &MAP.boxes {
        let wall = h.y > 2.0 && c.y - h.y < 0.0;
        if wall {
            let shade = stones[((c.x * 0.37 + c.z * 0.61).abs() as usize) % stones.len()].clone();
            commands.spawn((Mesh3d(meshes.add(textured_box(c, h, Vec2::splat(4.0), true))), MeshMaterial3d(shade.clone())));
            // The carved band just under the top, standing slightly proud of the wall.
            let top = c.y + h.y;
            let band = Vec3::new(c.x, top - 0.75, c.z);
            let half = Vec3::new(h.x + 0.05, 0.25, h.z + 0.05);
            commands.spawn((Mesh3d(meshes.add(textured_box(band, half, Vec2::new(4.0, 0.5), true))), MeshMaterial3d(frieze.clone())));
            // Cap stone ledge along the top.
            let cap = Vec3::new(c.x, top - 0.1, c.z);
            let half = Vec3::new(h.x + 0.12, 0.1, h.z + 0.12);
            commands.spawn((Mesh3d(meshes.add(textured_box(cap, half, Vec2::splat(4.0), true))), MeshMaterial3d(shade)));
        } else {
            // Crates: wooden ones and green ammo boxes, a crate per meter (stacked on the tall ones).
            let green = (c.x * 7.3 + c.z * 3.1).sin() > 0.2;
            let tile = Vec2::new(1.0, h.y * 2.0 / (h.y * 2.0).round().max(1.0));
            let mat = if green { ammo.clone() } else { wood.clone() };
            commands.spawn((Mesh3d(meshes.add(textured_box(c, h, tile, false))), MeshMaterial3d(mat)));
        }
    }

    // A darker base course along walls where they meet open ground, at that spot's floor height. Two shared meshes
    // (one per wall direction) since there are hundreds of them.
    let base_mat = mats.add(StandardMaterial { base_color: Color::srgb(0.78, 0.7, 0.62), ..mats.get(&stone).unwrap().clone() });
    let base_meshes = [
        meshes.add(textured_box(Vec3::ZERO, Vec3::new(0.05, 0.2, CELL / 2.0), Vec2::new(4.0, 4.0), false)),
        meshes.add(textured_box(Vec3::ZERO, Vec3::new(CELL / 2.0, 0.2, 0.05), Vec2::new(4.0, 4.0), false)),
    ];
    for z in 0..MAP.depth() {
        for x in 0..MAP.width() {
            if !MAP.is_wall(x, z) {
                continue;
            }
            for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz));
                if !MAP.walkable(nx, nz) {
                    continue;
                }
                let normal = Vec3::new(dx as f32, 0.0, dz as f32);
                let at = MAP.center(x, z) + normal * (CELL / 2.0 + 0.05) + Vec3::Y * (MAP.ground(nx, nz).y + 0.2);
                let mesh = base_meshes[if dx != 0 { 0 } else { 1 }].clone();
                commands.spawn((Mesh3d(mesh), MeshMaterial3d(base_mat.clone()), Transform::from_translation(at)));
            }
        }
    }

    // Arched doors at street level and shutters higher up, sprinkled on walls that face open ground.
    for z in 0..MAP.depth() {
        for x in 0..MAP.width() {
            if !MAP.is_wall(x, z) {
                continue;
            }
            for (side, (dx, dz)) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)].into_iter().enumerate() {
                let (nx, nz) = (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz));
                if !MAP.walkable(nx, nz) {
                    continue;
                }
                let roll = ((x * 73 + z * 151 + side * 37) as f32 * 12.9898).sin().fract().abs();
                let (size, lift) = match roll {
                    r if r < 0.04 => (Vec2::new(1.6, 2.6), 0.0),
                    r if r < 0.09 => (Vec2::new(1.0, 1.6), 2.4),
                    _ => continue,
                };
                let normal = Vec3::new(dx as f32, 0.0, dz as f32);
                let floor = MAP.ground(nx, nz).y;
                let at = MAP.center(x, z) + normal * (CELL / 2.0 + 0.03) + Vec3::Y * (floor + lift + size.y / 2.0);
                let half = if dx != 0 { Vec3::new(0.03, size.y / 2.0, size.x / 2.0) } else { Vec3::new(size.x / 2.0, size.y / 2.0, 0.03) };
                commands.spawn((Mesh3d(meshes.add(textured_box(at, half, size, false))), MeshMaterial3d(door.clone())));
                // A stone frame standing out from the wall: jambs, a lintel, and a sill under windows.
                let along = if dx != 0 { Vec3::Z } else { Vec3::X };
                let piece = |center: Vec3, len: f32, height: f32, depth: f32| {
                    let half = along * len + Vec3::Y * height + normal.abs() * depth;
                    (center + normal * depth, half)
                };
                let mut frame = vec![
                    piece(at + along * (size.x / 2.0 + 0.1), 0.1, size.y / 2.0, 0.07),
                    piece(at - along * (size.x / 2.0 + 0.1), 0.1, size.y / 2.0, 0.07),
                    piece(at + Vec3::Y * (size.y / 2.0 + 0.1), size.x / 2.0 + 0.22, 0.1, 0.09),
                ];
                if lift > 0.0 {
                    frame.push(piece(at - Vec3::Y * (size.y / 2.0 + 0.05), size.x / 2.0 + 0.16, 0.05, 0.13));
                }
                for (center, half) in frame {
                    commands.spawn((Mesh3d(meshes.add(textured_box(center, half, Vec2::splat(4.0), true))), MeshMaterial3d(stone.clone())));
                }
            }
        }
    }

    // Sky dome: pale and hazy at the horizon, deeper blue overhead, like the desert skybox.
    let mut sky = Sphere::new(900.0).mesh().uv(48, 24);
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = sky.attribute(Mesh::ATTRIBUTE_POSITION) {
        // Linear colors: roughly sRGB (0.78, 0.85, 0.92) at the horizon to (0.33, 0.55, 0.85) at the top.
        let colors: Vec<[f32; 4]> = p
            .iter()
            .map(|v| {
                let up = (v[1] / 900.0).clamp(0.0, 1.0).powf(0.5);
                let (h, z) = (Vec3::new(0.58, 0.69, 0.82), Vec3::new(0.09, 0.26, 0.69));
                let c = h.lerp(z, up);
                [c.x, c.y, c.z, 1.0]
            })
            .collect();
        sky.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    commands.spawn((
        Mesh3d(meshes.add(sky)),
        MeshMaterial3d(mats.add(StandardMaterial { unlit: true, fog_enabled: false, cull_mode: None, ..default() })),
        bevy::light::NotShadowCaster,
    ));
    // A cloud layer high overhead, outside the fog.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(2400.0, 2400.0))),
        MeshMaterial3d(mats.add(StandardMaterial {
            base_color_texture: Some(images.add(textures::clouds())),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            fog_enabled: false,
            cull_mode: None,
            uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(4.0)),
            ..default()
        })),
        Transform::from_xyz(0.0, 220.0, 0.0),
        bevy::light::NotShadowCaster,
    ));
    // Two rings of mountains: brown ones nearer, bluish ones behind, softened by the fog.
    // Vertex colors are linear: these are roughly sRGB (0.55, 0.45, 0.36) and (0.6, 0.6, 0.66).
    for (radius, low, high, color, seed) in [(230.0, 25.0, 70.0, [0.24, 0.15, 0.09], 1.0), (340.0, 50.0, 130.0, [0.3, 0.27, 0.27], 4.0)] {
        commands.spawn((
            Mesh3d(meshes.add(mountain_ring(radius, low, high, color, seed))),
            MeshMaterial3d(mats.add(StandardMaterial { unlit: true, fog_enabled: false, cull_mode: None, double_sided: true, ..default() })),
            bevy::light::NotShadowCaster,
        ));
    }
}

/// A box whose UVs are in meters divided by `tile`: anchored to the world (so neighboring walls line up) or to the
/// box itself (so a crate face shows whole crates).
fn textured_box(center: Vec3, half: Vec3, tile: Vec2, world: bool) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    // (normal, u axis, v axis) with u x v = normal, so corners in order are counter-clockwise from outside.
    let faces = [(Vec3::X, Vec3::NEG_Z, Vec3::Y), (Vec3::NEG_X, Vec3::Z, Vec3::Y), (Vec3::Z, Vec3::X, Vec3::Y), (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::Z, Vec3::X), (Vec3::NEG_Y, Vec3::X, Vec3::Z)];
    let (mut pos, mut normals, mut uvs, mut idx) = (vec![], vec![], vec![], vec![]);
    for (n, u, v) in faces {
        let base = pos.len() as u32;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let local = n * n.dot(half).abs() + u * su * u.dot(half).abs() + v * sv * v.dot(half).abs();
            let p = center + local;
            let anchor = if world { p } else { local + half };
            // Image rows run downwards, world up is +Y, hence the minus on v.
            let flip = if v == Vec3::Y { -1.0 } else { 1.0 };
            let (cu, cv) = if world { (anchor.dot(u), flip * anchor.dot(v)) } else { ((su + 1.0) * u.dot(half).abs(), (1.0 - sv) * v.dot(half).abs()) };
            pos.push(p.to_array());
            normals.push(n.to_array());
            uvs.push([cu / tile.x, cv / tile.y]);
        }
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(idx))
}

/// A jagged ring of peaks around the map, facing inwards.
fn mountain_ring(radius: f32, low: f32, high: f32, color: [f32; 3], seed: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    const SEGMENTS: usize = 160;
    let (mut pos, mut colors, mut idx) = (vec![], vec![], vec![]);
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let ridge = ((a * 3.0 + seed).sin() * 0.5 + (a * 7.0 + seed * 2.0).sin() * 0.3 + (a * 17.0 + seed).sin() * 0.2) * 0.5 + 0.5;
        let peak = low + (high - low) * ridge.powf(1.5);
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        pos.push((dir * radius + Vec3::Y * -10.0).to_array());
        pos.push((dir * (radius + peak * 0.6) + Vec3::Y * peak).to_array());
        // Rocky: faces alternate lit and shaded, darker at the foot.
        let rock = 0.8 + 0.35 * ((a * 41.0 + seed).sin() * 0.5 + 0.5);
        colors.push([color[0] * 0.6, color[1] * 0.6, color[2] * 0.6, 1.0]);
        colors.push([color[0] * rock, color[1] * rock, color[2] * rock, 1.0]);
    }
    for i in 0..SEGMENTS as u32 {
        let (b, t, nb, nt) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        idx.extend([b, t, nb, t, nt, nb]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(idx));
    mesh.compute_smooth_normals();
    mesh
}

/// For checking visuals without a person at the screen: `FPS16_SHOT=out.png` saves one screenshot after
/// `FPS16_SHOT_AT` seconds (default 4) and quits; `FPS16_POSE="x,y,z,yaw,pitch"` places the camera (degrees);
/// `FPS16_FIRE` keeps clicking fire.
#[derive(Resource)]
struct ShotTarget(Handle<Image>);

fn debug_shot(
    mut commands: Commands,
    target: Res<ShotTarget>,
    mut cam: Single<&mut Transform, With<Camera3d>>,
    time: Res<Time>,
    mut taken: Local<Option<f32>>,
) {
    use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};
    if let Ok(pose) = std::env::var("FPS16_POSE") {
        let v: Vec<f32> = pose.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if let [x, y, z, yaw, pitch] = v[..] {
            cam.translation = Vec3::new(x, y, z);
            cam.rotation = look_rot(yaw.to_radians(), pitch.to_radians());
        }
    }
    let now = time.elapsed_secs();
    match *taken {
        None if now > std::env::var("FPS16_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0) => {
            let path = std::env::var("FPS16_SHOT").unwrap_or_default();
            commands.spawn(Screenshot::image(target.0.clone())).observe(save_to_disk(path));
            *taken = Some(now);
        }
        Some(at) if now > at + 1.5 => std::process::exit(0),
        _ => {}
    }
}

/// Spawns this frame's muzzle flashes, impact puffs, bullet holes and blood.
fn spawn_fx(
    mut commands: Commands,
    mut g: ResMut<Game>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut decals: ResMut<Decals>,
    mut puff_mesh: Local<Option<Handle<Mesh>>>,
    mut hole: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
) {
    let puff = puff_mesh.get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap())).clone();
    let (hole_mesh, hole_mat) = hole
        .get_or_insert_with(|| {
            let m = meshes.add(Plane3d::default().mesh().size(0.09, 0.09));
            let mat = mats.add(StandardMaterial { base_color: Color::srgb(0.08, 0.06, 0.05), unlit: true, ..default() });
            (m, mat)
        })
        .clone();
    let mut fading = |color: Color| mats.add(StandardMaterial { base_color: color, unlit: true, alpha_mode: AlphaMode::Blend, ..default() });
    for fx in std::mem::take(&mut g.fx) {
        match fx {
            FxKind::Muzzle(at) => {
                let material = fading(Color::srgba(1.0, 0.85, 0.4, 0.9));
                commands.spawn((
                    Mesh3d(puff.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(at).with_scale(Vec3::splat(0.07)),
                    Fx { age: 0.0, life: 0.05, grow: 1.5, size: 0.07, material: Some(material) },
                    bevy::light::NotShadowCaster,
                ));
                commands.spawn((
                    PointLight { intensity: 60_000.0, range: 8.0, color: Color::srgb(1.0, 0.8, 0.5), ..default() },
                    Transform::from_translation(at),
                    Fx { age: 0.0, life: 0.05, grow: 0.0, size: 1.0, material: None },
                ));
            }
            FxKind::Impact(at, normal) => {
                let material = fading(Color::srgba(0.75, 0.62, 0.45, 0.8));
                commands.spawn((
                    Mesh3d(puff.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(at + normal * 0.08).with_scale(Vec3::splat(0.06)),
                    Fx { age: 0.0, life: 0.45, grow: 5.0, size: 0.06, material: Some(material) },
                    bevy::light::NotShadowCaster,
                ));
                let rot = Quat::from_rotation_arc(Vec3::Y, normal);
                let e = commands
                    .spawn((Mesh3d(hole_mesh.clone()), MeshMaterial3d(hole_mat.clone()), Transform::from_translation(at + normal * 0.012).with_rotation(rot)))
                    .id();
                decals.0.push_back(e);
                if decals.0.len() > 80 {
                    commands.entity(decals.0.pop_front().unwrap()).despawn();
                }
            }
            FxKind::Blood(at) => {
                let material = fading(Color::srgba(0.6, 0.02, 0.02, 0.85));
                commands.spawn((
                    Mesh3d(puff.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(at).with_scale(Vec3::splat(0.08)),
                    Fx { age: 0.0, life: 0.3, grow: 3.5, size: 0.08, material: Some(material) },
                    bevy::light::NotShadowCaster,
                ));
            }
        }
    }
}

fn update_fx(
    mut commands: Commands,
    mut q: Query<(Entity, &mut Fx, &mut Transform, Option<&mut PointLight>)>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
) {
    for (e, mut fx, mut t, light) in &mut q {
        let dt = time.delta_secs();
        fx.age += dt;
        let k = fx.age / fx.life;
        if k >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        t.scale = Vec3::splat(fx.size * (1.0 + fx.grow * k));
        if let Some(mut m) = fx.material.as_ref().and_then(|h| mats.get_mut(h)) {
            m.base_color.set_alpha((1.0 - k) * 0.85);
        }
        if let Some(mut l) = light {
            l.intensity *= 1.0 - k;
        }
    }
}

/// CS 1.6 scoreboard while Tab is held: per team, sorted by kills, with deaths and latency; dead players, bots and
/// the bomb carrier marked.
fn score_panel(
    keys: Res<ButtonInput<KeyCode>>,
    mut panel: Single<&mut Visibility, With<ScorePanel>>,
    mut sections: Query<(&ScoreSection, &mut Text)>,
    g: Res<Game>,
) {
    let show = keys.pressed(KeyCode::Tab) || std::env::var_os("FPS16_SCOREBOARD").is_some();
    **panel = if show { Visibility::Inherited } else { Visibility::Hidden };
    if !show {
        return;
    }
    let carrier = match g.round.map(|r| r.bomb) {
        Some(Bomb::Carried(id)) => Some(id),
        _ => None,
    };
    for (section, mut text) in &mut sections {
        let team = section.0;
        let score = g.round.map_or(0, |r| r.score[team as usize]);
        let mut rows: Vec<&NetPlayer> = g.players.iter().filter(|p| p.team == team).collect();
        rows.sort_by_key(|p| (std::cmp::Reverse(p.kills), p.deaths));
        let mut out = format!("{:<34}{:>6}{:>7}{:>9}\n", format!("{}  -  {score}", team.name()), "Score", "Deaths", "Latency");
        for p in rows {
            let tag = match (p.hp <= 0, Some(p.id) == carrier) {
                (true, _) => "Dead",
                (_, true) => "Bomb",
                _ => "",
            };
            let you = if Some(p.id) == g.me { ">" } else { " " };
            let ping = if p.bot { "BOT".to_string() } else { p.ping.to_string() };
            out += &format!("{you}{:<20}{tag:<13}{:>6}{:>7}{:>9}\n", p.name, p.kills, p.deaths, ping);
        }
        text.0 = out;
    }
}

/// Round-end and bomb announcements in big letters for a few seconds.
fn announce(mut text: Single<&mut Text, With<AnnounceText>>, g: Res<Game>) {
    text.0 = match &g.announce {
        Some((line, at)) if at.elapsed().as_secs_f32() < 3.5 => line.clone(),
        _ => String::new(),
    };
}

fn chat(mut text: Single<&mut Text, With<ChatText>>, mut g: ResMut<Game>) {
    g.chat.retain(|(_, at)| at.elapsed().as_secs_f32() < 7.0);
    text.0 = g.chat.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n");
}

fn radio_menu(mut vis: Single<&mut Visibility, With<RadioMenu>>, g: Res<Game>) {
    **vis = if g.radio_menu || std::env::var_os("FPS16_RADIOMENU").is_some() { Visibility::Inherited } else { Visibility::Hidden };
}

/// C4: an olive brick of charges taped together, keypad and display on top, a couple of wires and a red light.
fn spawn_c4(commands: &mut Commands, meshes: &mut Assets<Mesh>, mats: &mut Assets<StandardMaterial>) {
    let mut part = |size: [f32; 3], at: [f32; 3], color: Color| {
        (Mesh3d(meshes.add(Cuboid::from_size(Vec3::from(size)))), MeshMaterial3d(mats.add(color)), Transform::from_translation(Vec3::from(at)))
    };
    let parts = [
        part([0.34, 0.08, 0.2], [0.0, 0.0, 0.0], Color::srgb(0.36, 0.37, 0.25)),
        part([0.345, 0.082, 0.03], [0.0, 0.0, -0.06], Color::srgb(0.45, 0.38, 0.22)),
        part([0.345, 0.082, 0.03], [0.0, 0.0, 0.06], Color::srgb(0.45, 0.38, 0.22)),
        part([0.13, 0.015, 0.1], [0.05, 0.046, 0.0], Color::srgb(0.12, 0.12, 0.12)),
        part([0.09, 0.008, 0.035], [0.05, 0.056, -0.025], Color::srgb(0.25, 0.45, 0.2)),
        part([0.02, 0.012, 0.16], [-0.1, 0.046, 0.0], Color::srgb(0.7, 0.1, 0.1)),
        part([0.02, 0.012, 0.16], [-0.07, 0.046, 0.0], Color::srgb(0.15, 0.2, 0.6)),
    ];
    let led_mat = mats.add(StandardMaterial { base_color: Color::srgb(0.6, 0.0, 0.0), emissive: LinearRgba::rgb(0.3, 0.0, 0.0), ..default() });
    let led = commands
        .spawn((
            BombLed(led_mat.clone()),
            Mesh3d(meshes.add(Sphere::new(0.012))),
            MeshMaterial3d(led_mat),
            Transform::from_xyz(0.1, 0.056, 0.03),
            PointLight { intensity: 0.0, range: 3.0, color: Color::srgb(1.0, 0.1, 0.05), ..default() },
        ))
        .id();
    let ids: Vec<Entity> = parts.into_iter().map(|p| commands.spawn(p).id()).collect();
    commands.spawn((BombModel, Transform::default(), Visibility::Hidden)).add_children(&ids).add_child(led);
}

fn target_id(mut q: Single<(&mut Text, &mut TextColor), With<TargetText>>, g: Res<Game>) {
    let (text, color) = &mut *q;
    let my_team = g.me.and_then(|me| g.teams.get(&me)).copied();
    let player = |id: u32| g.players.iter().find(|p| p.id == id);
    let (line, team) = match (g.hp > 0, g.aim_target, g.spectate) {
        (true, Some(id), _) => match player(id) {
            Some(p) if Some(p.team) == my_team => (format!("Friend: {}   Health: {}%", p.name, p.hp), Some(p.team)),
            Some(p) => (format!("Enemy: {}", p.name), Some(p.team)),
            None => (String::new(), None),
        },
        (false, _, Some((id, ..))) if spectating(&g) => (format!("Spectating {}", name(&g, id)), my_team),
        _ => (String::new(), None),
    };
    text.0 = line;
    color.0 = team_text_color(team);
}

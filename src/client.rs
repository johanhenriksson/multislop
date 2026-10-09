use crate::map::{CELL, MAP};
use crate::model::{Rig, barrel_length, spawn_soldier, spawn_weapon, viewmodel_offset};
use crate::weapons::{ALL_WEAPONS, Weapon, buy_menu};
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
    /// Clock time of the next bomb beep.
    next_beep: f64,
    /// Where and when (clock) the bomb went off, for the flash.
    explosion: Option<(Vec3, f64)>,
    /// The teammate we watch while dead: id, interpolated eye position and view.
    spectate: Option<(u32, Vec3, f32, f32)>,
    /// Interpolated position of a remote bomb carrier.
    carrier_pos: Option<Vec3>,
    armor: u8,
    money: u32,
    names: HashMap<u32, String>,
    /// Scoped in with the AWP.
    zoom: bool,
    /// Buy menu open.
    menu: bool,
    /// Key presses caught each frame, sent with the next input.
    want_slot: u8,
    want_buy: u8,
    want_reload: bool,
    kill_feed: VecDeque<(String, Instant)>,
    hurt_at: Option<Instant>,
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
}

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

#[derive(Component)]
struct HurtFlash;

/// Sniper scope overlay shown while zoomed with the AWP.
#[derive(Component)]
struct Scope;

#[derive(Component)]
struct Scoreboard;

#[derive(Component)]
struct BombModel;

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

#[derive(Component)]
struct Crosshair;

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
        .add_systems(PreUpdate, receive)
        .add_systems(FixedUpdate, net_tick)
        .add_systems(
            Update,
            (
                grab_cursor,
                look,
                controls,
                (render_remotes, render_self, viewmodel, render_bomb, draw_tracers, bomb_beep, play_sounds),
                (hud, scoreboard, ammo_hud, buy_menu_hud, kill_feed, death_fade, hurt_flash, crosshair, scope),
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
    });
    // First-person weapons; each barrel tip lines up with `muzzle`.
    let viewmodels: Vec<Entity> = ALL_WEAPONS
        .into_iter()
        .map(|w| {
            let e = spawn_weapon(&mut commands, &mut meshes, &mut mats, w, Transform::from_translation(viewmodel_offset(w)));
            commands.entity(e).insert(Viewmodel(w)).id()
        })
        .collect();
    commands
        .spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: FOV, ..default() })))
        .add_children(&viewmodels);
    commands.spawn((
        DirectionalLight { illuminance: 8000.0, shadow_maps_enabled: true, ..default() },
        Transform::default().looking_to(Vec3::new(-0.4, -1.0, -0.3), Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() });
    commands.spawn((Mesh3d(meshes.add(terrain_mesh())), MeshMaterial3d(mats.add(Color::WHITE))));
    let crate_mat = mats.add(Color::srgb(0.55, 0.4, 0.22));
    let wall_mat = mats.add(Color::srgb(0.82, 0.72, 0.55));
    for &(c, h) in &MAP.boxes {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::from_size(h * 2.0))),
            MeshMaterial3d(if h.y > 2.0 { wall_mat.clone() } else { crate_mat.clone() }),
            Transform::from_translation(c),
        ));
    }
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
    commands.spawn((
        BombModel,
        Mesh3d(meshes.add(Cuboid::new(0.4, 0.15, 0.3))),
        MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.9, 0.1, 0.1), unlit: true, ..default() })),
        Transform::default(),
        Visibility::Hidden,
    ));
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
        children![(Crosshair, Text::new("+"))],
    ));
    commands.spawn((
        Hud,
        Text::new(""),
        Node { position_type: PositionType::Absolute, left: px(12), bottom: px(12), ..default() },
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
        TextFont { font_size: FontSize::Px(28.0), ..default() },
        Node { position_type: PositionType::Absolute, right: px(16), bottom: px(12), ..default() },
    ));
    commands.spawn((
        BuyMenu,
        Text::new(""),
        Node { position_type: PositionType::Absolute, left: px(24), top: percent(30), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Visibility::Hidden,
    ));
    commands.spawn((
        KillFeed,
        Text::new(""),
        TextLayout { justify: Justify::Right, ..default() },
        Node { position_type: PositionType::Absolute, right: px(16), top: px(12), ..default() },
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
    if let Some(n) = digits.iter().position(|&k| keys.just_pressed(k)) {
        if g.menu {
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
        use_key: keys.pressed(KeyCode::KeyE),
        fire: active && !g.menu && mouse.pressed(MouseButton::Left),
        slot: std::mem::take(&mut g.want_slot),
        reload: std::mem::take(&mut g.want_reload),
        zoom: g.zoom,
        buy: std::mem::take(&mut g.want_buy),
        view_tick: (server_tick(&g) - g.interp_ticks) as f32,
    };

    g.prev_pos = g.pred.pos;
    if g.me.is_some() && g.hp > 0 {
        let mut pred = g.pred;
        let was_reloading = pred.reloading > 0;
        if let Some((weapon, d)) = step_player(&mut pred, &inp) {
            let o = eye(&pred);
            g.sounds.push((Sound::Gun(weapon), 0.6));
            add_tracer(&mut g, weapon, muzzle(o, d, weapon), o + d * trace_world(o, d));
            if weapon == Weapon::Awp {
                g.zoom = false;
            }
        }
        if pred.reloading > 0 && !was_reloading {
            g.sounds.push((Sound::Reload, 0.5));
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
        let packet = InputPacket { build: BUILD.into(), name: player_name(), time: clock(), inputs };
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
    for p in &snap.players {
        g.teams.insert(p.id, p.team);
        if g.names.get(&p.id) != Some(&p.name) {
            g.names.insert(p.id, p.name.clone());
        }
    }
    for h in &snap.hits {
        if Some(h.attacker) == g.me {
            g.sounds.push((if h.headshot { Sound::Headshot } else { Sound::HitBody }, 0.7));
            if h.killed {
                g.sounds.push((Sound::Kill, 0.6));
            }
        }
        if Some(h.victim) == g.me {
            g.sounds.push((Sound::Hurt, 0.8));
            g.hurt_at = Some(Instant::now());
        }
        if h.killed {
            let hs = if h.headshot { " (headshot)" } else { "" };
            let line = format!("{} [{}] {}{hs}", name(g, h.attacker), h.weapon.spec().name, name(g, h.victim));
            g.kill_feed.push_back((line, Instant::now()));
            if g.kill_feed.len() > 5 {
                g.kill_feed.pop_front();
            }
        }
    }
    let prev = g.round.replace(snap.round);
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
        // Hide on death as soon as we hear of it, and stay hidden until the respawn is interpolated past, so the
        // respawn teleport is never drawn as a slide across the map.
        let alive_in = |s: &Snapshot| s.players.iter().find(|q| q.id == p.id).is_some_and(|q| q.hp > 0);
        let alive = alive_in(newest) && alive_in(a) && p.hp > 0;
        if alive && spectate.is_none() && Some(p.team) == my_team {
            spectate = Some((p.id, pos + Vec3::Y * EYE, yaw, pitch));
        }
        if Some(p.id) == carrier {
            carrier_pos = Some(pos);
        }
        seen.insert(p.id, (pos, yaw, pitch, alive, p.state.weapon()));
    }
    (g.spectate, g.carrier_pos) = (spectate, carrier_pos);
    // We're looking out of their eyes, so their own model would only block the view.
    let watched = spectate.filter(|_| spectating(&g)).map(|s| s.0);

    for (e, r, mut rig, mut t, mut vis) in &mut q {
        let Some((pos, yaw, pitch, alive, weapon)) = seen.remove(&r.0) else {
            commands.entity(e).despawn();
            continue;
        };
        t.translation = pos;
        t.rotation = Quat::from_rotation_y(yaw);
        *vis = if alive && watched != Some(r.0) { Visibility::Inherited } else { Visibility::Hidden };
        let swing = rig.animate(pos, time.delta_secs());
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

fn hud(mut text: Single<&mut Text, With<Hud>>, mut g: ResMut<Game>, time: Res<Time>) {
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
        _ if g.hp > 0 => format!("HP {}   Armor {}   ${}", g.hp, g.armor, g.money),
        _ => format!("DEAD   ${}", g.money),
    };
    text.0 = format!("{status}   K {}  D {}\n{}", g.kills, g.deaths, g.net_graph.line);
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
    g: Res<Game>,
    mut gizmos: Gizmos,
) {
    let (t, vis) = &mut *bomb;
    let at = match g.round.map(|r| r.bomb) {
        Some(Bomb::Dropped(p) | Bomb::Planted(p)) => Some(p + Vec3::Y * 0.08),
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
    g.next_beep = clock() + 0.12 + 0.88 * (time_left / BOMB_SECS) as f64;
    let volume = 0.7 / (1.0 + pos.distance(g.pred.pos) / 15.0);
    g.sounds.push((Sound::Beep, volume));
}

fn scoreboard(mut text: Single<&mut Text, With<Scoreboard>>, g: Res<Game>) {
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
    text.0 = format!("T {}   {}:{:02}   {} CT\n{side}\n{message}", r.score[0], secs / 60, secs % 60, r.score[1]);
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

/// Grayed out while reloading or drawing, so a held trigger firing late doesn't read as lag. Hidden when scoped.
fn crosshair(mut q: Single<(&mut TextColor, &mut Visibility), With<Crosshair>>, g: Res<Game>) {
    let (color, vis) = &mut *q;
    let busy = g.pred.reloading > 0 || g.pred.fire_cd as f32 > g.pred.weapon().spec().cycle * TICK_HZ as f32;
    color.0 = if busy { Color::srgba(0.5, 0.5, 0.5, 0.6) } else { Color::WHITE };
    **vis = if g.zoom { Visibility::Hidden } else { Visibility::Inherited };
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

/// Shows the held weapon, dips it while reloading, kicks it back on recoil; hidden when dead or scoped.
fn viewmodel(mut q: Query<(&Viewmodel, &mut Transform, &mut Visibility)>, g: Res<Game>) {
    let held = g.pred.weapon();
    let reload = g.pred.reloading as f32 / (held.spec().reload * TICK_HZ as f32).max(1.0);
    let dip = (reload * std::f32::consts::PI).sin();
    let show = g.hp > 0 && g.me.is_some() && !g.zoom;
    for (vm, mut t, mut vis) in &mut q {
        *vis = if show && vm.0 == held { Visibility::Inherited } else { Visibility::Hidden };
        t.translation = viewmodel_offset(vm.0) + Vec3::new(0.0, -0.15 * dip, 0.03 * g.pred.recoil.min(3.0));
        t.rotation = Quat::from_rotation_x(-0.6 * dip);
    }
}

fn ammo_hud(mut text: Single<&mut Text, With<AmmoText>>, g: Res<Game>) {
    let w = g.pred.weapon();
    text.0 = match g.pred.ammo() {
        _ if g.hp <= 0 => String::new(),
        None => w.spec().name.into(),
        Some(_) if g.pred.reloading > 0 => format!("{}  reloading...", w.spec().name),
        Some((mag, reserve)) => format!("{}  {mag} / {reserve}", w.spec().name),
    };
}

fn buy_menu_hud(mut q: Single<(&mut Text, &mut Visibility), With<BuyMenu>>, g: Res<Game>) {
    let (text, vis) = &mut *q;
    **vis = if g.menu { Visibility::Inherited } else { Visibility::Hidden };
    let (Some(r), Some(team)) = (g.round, g.me.and_then(|me| g.teams.get(&me))) else { return };
    let mut lines = format!(" BUY  ${}   ({:.0} s left)\n", g.money, r.buy_secs);
    for (i, item) in buy_menu(*team).iter().enumerate() {
        let mark = if item.price() <= g.money { " " } else { "x" };
        lines += &format!(" {mark} {}  {:<16} ${}\n", i + 1, item.name(), item.price());
    }
    text.0 = lines + " B / Esc to close ";
}

fn kill_feed(mut text: Single<&mut Text, With<KillFeed>>, mut g: ResMut<Game>) {
    g.kill_feed.retain(|(_, at)| at.elapsed().as_secs_f32() < 6.0);
    text.0 = g.kill_feed.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n");
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
    let (mut pos, mut normals, mut colors, mut idx) = (vec![], vec![], vec![], vec![]);
    for z in 0..MAP.depth() {
        for x in 0..MAP.width() {
            let corner = |cx: usize, cz: usize| {
                Vec3::new(cx as f32 * CELL - w / 2.0, MAP.corner(cx, cz), cz as f32 * CELL - d / 2.0)
            };
            let quad = [corner(x, z), corner(x + 1, z), corner(x + 1, z + 1), corner(x, z + 1)];
            let normal = (quad[2] - quad[0]).cross(quad[1] - quad[3]).normalize_or(Vec3::Y);
            let normal = if normal.y < 0.0 { -normal } else { normal };
            let base = pos.len() as u32;
            for p in quad {
                let shade = 0.55 + 0.45 * (p.y / 2.0).clamp(0.0, 1.0);
                pos.push(p.to_array());
                normals.push(normal.to_array());
                colors.push([0.78 * shade, 0.67 * shade, 0.48 * shade, 1.0]);
            }
            idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(idx))
}

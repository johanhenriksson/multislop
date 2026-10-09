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
const BEAM_SECS: f32 = 0.4;

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
    /// Shot beams: (from, to, seconds left, color).
    tracers: Vec<(Vec3, Vec3, f32, Color)>,
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
}

#[derive(Clone, Copy)]
enum Sound {
    Shoot,
    Hit,
    Death,
}

#[derive(Resource)]
struct Sounds {
    shoot: Handle<AudioSource>,
    hit: Handle<AudioSource>,
    death: Handle<AudioSource>,
}

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
        .add_systems(Update, (grab_cursor, look, (render_self, render_remotes, hud, draw_tracers, play_sounds, death_fade, crosshair)).chain())
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut audio: ResMut<Assets<AudioSource>>,
) {
    let mut sound = |wav: Vec<u8>| audio.add(AudioSource { bytes: wav.into() });
    commands.insert_resource(Sounds { shoot: sound(sfx::shoot()), hit: sound(sfx::hit()), death: sound(sfx::death()) });
    commands
        .spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: 1.6, ..default() })))
        .with_child((
            Mesh3d(meshes.add(Cuboid::new(0.08, 0.1, 0.6))),
            MeshMaterial3d(mats.add(Color::srgb(0.15, 0.15, 0.15))),
            Transform::from_xyz(0.25, -0.22, -0.45),
        ));
    commands.spawn((
        DirectionalLight { illuminance: 8000.0, shadow_maps_enabled: true, ..default() },
        Transform::default().looking_to(Vec3::new(-0.4, -1.0, -0.3), Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(FLOOR[0] + 2.0, FLOOR[1] + 2.0))),
        MeshMaterial3d(mats.add(Color::srgb(0.55, 0.5, 0.4))),
    ));
    let crate_mat = mats.add(Color::srgb(0.6, 0.45, 0.25));
    let wall_mat = mats.add(Color::srgb(0.7, 0.68, 0.6));
    for (i, (c, h)) in BOXES.iter().enumerate() {
        let size = Vec3::from(*h) * 2.0;
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::from_size(size))),
            MeshMaterial3d(if i < 4 { wall_mat.clone() } else { crate_mat.clone() }),
            Transform::from_translation(Vec3::from(*c)),
        ));
    }
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
    g.yaw -= motion.delta.x * MOUSE_SENS;
    g.pitch = (g.pitch - motion.delta.y * MOUSE_SENS).clamp(-1.55, 1.55);
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
        fire: active && mouse.pressed(MouseButton::Left),
        view_tick: (server_tick(&g) - g.interp_ticks) as f32,
    };

    g.prev_pos = g.pred.pos;
    if g.me.is_some() && g.hp > 0 {
        let mut pred = g.pred;
        if step_player(&mut pred, &inp) {
            let (o, d) = (eye(&pred), aim_dir(&pred));
            let color = player_color(g.me.unwrap());
            g.sounds.push((Sound::Shoot, 0.6));
            g.tracers.push((muzzle(o, d), o + d * trace_world(o, d), BEAM_SECS, color));
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
        let packet = InputPacket { build: BUILD.into(), time: clock(), inputs };
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
        if me.kills > g.kills {
            g.sounds.push((Sound::Hit, 0.8));
        }
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
        g.tracers.push((muzzle(s.from, d), s.to, BEAM_SECS, player_color(s.shooter)));
        let volume = 0.6 / (1.0 + s.from.distance(g.pred.pos) / 8.0);
        g.sounds.push((Sound::Shoot, volume));
    }
    g.snapshots.push_back(snap);
    while g.snapshots.len() > 64 {
        g.snapshots.pop_front();
    }
}

fn render_self(
    mut cam: Single<&mut Transform, With<Camera3d>>,
    mut g: ResMut<Game>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
) {
    g.smooth *= (-time.delta_secs() * 15.0).exp();
    let pos = g.prev_pos.lerp(g.pred.pos, fixed.overstep_fraction()) + g.smooth;
    cam.translation = pos + Vec3::Y * EYE;
    cam.rotation = look_rot(g.yaw, g.pitch);
}

fn render_remotes(
    mut commands: Commands,
    mut q: Query<(Entity, &Remote, &mut Transform, &mut Visibility)>,
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

    let mut seen = HashMap::new();
    for p in b.players.iter().filter(|p| Some(p.id) != g.me) {
        let from = a.players.iter().find(|q| q.id == p.id).map_or(p.state, |q| q.state);
        let pos = from.pos.lerp(p.state.pos, f) + p.state.vel * extrapolate;
        let yaw = from.yaw + wrap_angle(p.state.yaw - from.yaw) * f;
        // Hide on death as soon as we hear of it, and stay hidden until the respawn is interpolated past, so the
        // respawn teleport is never drawn as a slide across the map.
        let alive_in = |s: &Snapshot| s.players.iter().find(|q| q.id == p.id).is_some_and(|q| q.hp > 0);
        let alive = alive_in(newest) && alive_in(a) && p.hp > 0;
        seen.insert(p.id, (pos, yaw, alive));
    }

    for (e, r, mut t, mut vis) in &mut q {
        match seen.remove(&r.0) {
            Some((pos, yaw, alive)) => {
                t.translation = pos + Vec3::Y * HEIGHT / 2.0;
                t.rotation = Quat::from_rotation_y(yaw);
                *vis = if alive { Visibility::Inherited } else { Visibility::Hidden };
            }
            None => commands.entity(e).despawn(),
        }
    }
    for (id, (pos, yaw, _)) in seen {
        commands
            .spawn((
                Remote(id),
                Mesh3d(meshes.add(Capsule3d::new(RADIUS, HEIGHT - 2.0 * RADIUS))),
                MeshMaterial3d(mats.add(player_color(id))),
                Transform::from_translation(pos + Vec3::Y * HEIGHT / 2.0).with_rotation(Quat::from_rotation_y(yaw)),
            ))
            .with_child((
                Mesh3d(meshes.add(Cuboid::new(0.3, 0.12, 0.2))),
                MeshMaterial3d(mats.add(Color::BLACK)),
                Transform::from_xyz(0.0, EYE - HEIGHT / 2.0, -RADIUS),
            ));
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
        _ if g.hp > 0 => format!("HP {}", g.hp),
        _ => "DEAD".into(),
    };
    text.0 = format!("{status}   K {}  D {}\n{}", g.kills, g.deaths, g.net_graph.line);
}

fn draw_tracers(mut gizmos: Gizmos, mut g: ResMut<Game>, time: Res<Time>) {
    for (a, b, ttl, color) in &mut g.tracers {
        gizmos.line(*a, *b, color.with_alpha(*ttl / BEAM_SECS));
        *ttl -= time.delta_secs();
    }
    g.tracers.retain(|t| t.2 > 0.0);
}

/// Gun barrel tip for a shot from eye `o` along `d`, matching the first-person gun model, so your own beam is visible.
fn muzzle(o: Vec3, d: Vec3) -> Vec3 {
    let right = d.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(d);
    o + right * 0.25 - up * 0.22 + d * 0.75
}

fn player_color(id: u32) -> Color {
    Color::hsl((id * 67 % 360) as f32, 0.7, 0.5)
}

fn play_sounds(mut commands: Commands, mut g: ResMut<Game>, sounds: Res<Sounds>) {
    for (sound, volume) in g.sounds.drain(..) {
        let handle = match sound {
            Sound::Shoot => &sounds.shoot,
            Sound::Hit => &sounds.hit,
            Sound::Death => &sounds.death,
        };
        commands.spawn((
            AudioPlayer::new(handle.clone()),
            PlaybackSettings { volume: Volume::Linear(volume), ..PlaybackSettings::DESPAWN },
        ));
    }
}

fn death_fade(mut fade: Single<&mut BackgroundColor, With<DeathFade>>, g: Res<Game>) {
    let alpha = g.died_at.map_or(0.0, |t| (t.elapsed().as_secs_f32() / 0.8).min(1.0));
    fade.0 = Color::BLACK.with_alpha(alpha);
}

/// Grayed out while the weapon cools down, so a held trigger firing late doesn't read as lag.
fn crosshair(mut color: Single<&mut TextColor, With<Crosshair>>, g: Res<Game>) {
    color.0 = if g.pred.fire_cd > 0 { Color::srgba(0.5, 0.5, 0.5, 0.6) } else { Color::WHITE };
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

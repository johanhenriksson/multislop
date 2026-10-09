use crate::map::{CELL, MAP};
use crate::model::{Rig, spawn_rifle, spawn_soldier};
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
}

#[derive(Clone, Copy)]
enum Sound {
    Shoot,
    Hit,
    Death,
    Beep,
    Explosion,
}

#[derive(Resource)]
struct Sounds {
    shoot: Handle<AudioSource>,
    hit: Handle<AudioSource>,
    death: Handle<AudioSource>,
    beep: Handle<AudioSource>,
    explosion: Handle<AudioSource>,
}

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
            (grab_cursor, look, (render_remotes, render_self, render_bomb, hud, scoreboard, draw_tracers, bomb_beep, play_sounds, death_fade, crosshair))
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
        shoot: sound(sfx::shoot()),
        hit: sound(sfx::hit()),
        death: sound(sfx::death()),
        beep: sound(sfx::beep()),
        explosion: sound(sfx::explosion()),
    });
    // First-person rifle; its barrel tip lines up with `muzzle`.
    let rifle = spawn_rifle(&mut commands, &mut meshes, &mut mats, Transform::from_xyz(0.25, -0.22, -0.2));
    commands
        .spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: 1.6, ..default() })))
        .add_child(rifle);
    commands.spawn((
        DirectionalLight { illuminance: 8000.0, shadow_maps_enabled: true, ..default() },
        Transform::default().looking_to(Vec3::new(-0.4, -1.0, -0.3), Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(MAP.size().0, MAP.size().1))),
        MeshMaterial3d(mats.add(Color::srgb(0.76, 0.66, 0.48))),
    ));
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
        commands.spawn((Mesh3d(site_mesh.clone()), MeshMaterial3d(site_mat.clone()), Transform::from_translation(p + Vec3::Y * 0.02)));
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
        use_key: keys.pressed(KeyCode::KeyE),
        fire: active && mouse.pressed(MouseButton::Left),
        view_tick: (server_tick(&g) - g.interp_ticks) as f32,
    };

    g.prev_pos = g.pred.pos;
    if g.me.is_some() && g.hp > 0 {
        let mut pred = g.pred;
        if step_player(&mut pred, &inp) {
            let (o, d) = (eye(&pred), aim_dir(&pred));
            let color = team_color(g.teams.get(&g.me.unwrap()).copied());
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
    for p in &snap.players {
        g.teams.insert(p.id, p.team);
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
        g.tracers.push((muzzle(s.from, d), s.to, BEAM_SECS, team_color(g.teams.get(&s.shooter).copied())));
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
    if let Some((_, eye, yaw, pitch)) = g.spectate.filter(|_| spectating(&g)) {
        cam.translation = eye;
        cam.rotation = look_rot(yaw, pitch);
        return;
    }
    let pos = g.prev_pos.lerp(g.pred.pos, fixed.overstep_fraction()) + g.smooth;
    cam.translation = pos + Vec3::Y * EYE;
    cam.rotation = look_rot(g.yaw, g.pitch);
}

fn render_remotes(
    mut commands: Commands,
    mut q: Query<(Entity, &Remote, &mut Rig, &mut Transform, &mut Visibility)>,
    mut parts: Query<&mut Transform, Without<Remote>>,
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
        seen.insert(p.id, (pos, yaw, pitch, alive, p.team));
    }
    (g.spectate, g.carrier_pos) = (spectate, carrier_pos);
    // We're looking out of their eyes, so their own model would only block the view.
    let watched = spectate.filter(|_| spectating(&g)).map(|s| s.0);

    for (e, r, mut rig, mut t, mut vis) in &mut q {
        let Some((pos, yaw, pitch, alive, _)) = seen.remove(&r.0) else {
            commands.entity(e).despawn();
            continue;
        };
        t.translation = pos;
        t.rotation = Quat::from_rotation_y(yaw);
        *vis = if alive && watched != Some(r.0) { Visibility::Inherited } else { Visibility::Hidden };
        let swing = rig.animate(pos, time.delta_secs());
        for (leg, dir) in rig.legs.into_iter().zip([1.0, -1.0]) {
            if let Ok(mut leg) = parts.get_mut(leg) {
                leg.rotation = Quat::from_rotation_x(swing * dir);
            }
        }
        if let Ok(mut arms) = parts.get_mut(rig.arms) {
            arms.rotation = Quat::from_rotation_x(pitch);
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

fn team_color(team: Option<Team>) -> Color {
    match team {
        Some(Team::T) => Color::srgb(0.9, 0.6, 0.2),
        Some(Team::Ct) => Color::srgb(0.25, 0.45, 0.95),
        None => Color::WHITE,
    }
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
            Sound::Shoot => &sounds.shoot,
            Sound::Hit => &sounds.hit,
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

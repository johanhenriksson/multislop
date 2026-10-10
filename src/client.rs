//! The client host: window, input, audio and rendering for whatever game is loaded, with the netcode in between.
//! The game draws everything game related; the host only adds its own overlays (connection status, net graph).

use crate::net::*;
use crate::netclient::{NetClient, clock};
use crate::render::{Clicked, Ids, Renderer, color};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use bevy::winit::WinitSettings;
use sdk::client::{ClientGame, ClientSnapshot, FrameIn, Key, RawInput};
use sdk::{TICK_HZ, decode, encode};
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub type Game = Box<dyn ClientGame + Send + Sync>;

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

#[derive(Resource)]
struct Client {
    game: Game,
    netcode: NetClient,
    /// Got packets we couldn't decode: the server runs a different build.
    incompatible: bool,
    /// The server rejected us: it runs this other build.
    wrong_build: Option<String>,
}

/// Connection status, centered.
#[derive(Component)]
struct StatusText;

/// Network stats like CS's net_graph, toggled with N.
#[derive(Component)]
struct NetGraphText;

/// Frame stats for the net_graph line, next to `netcode.graph`.
#[derive(Default)]
struct FrameStats {
    frames: u32,
    worst_frame_ms: f32,
    since: Option<Instant>,
}

pub fn run(server: SocketAddr, game: Game, lag_ms: u64, jitter_ms: u64, interp_ms: u64) {
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
        .init_resource::<Clicked>()
        .insert_resource(Client { game, netcode: NetClient::new(interp_ms), incompatible: false, wrong_build: None })
        .add_systems(Startup, (setup, start).chain())
        .add_systems(PreUpdate, receive)
        .add_systems(FixedUpdate, net_tick)
        .add_systems(Update, (grab_cursor, frame, crate::render::anchor_ui, overlays).chain())
        .add_systems(PostUpdate, debug_shot.run_if(|| std::env::var_os("FPS16_SHOT").is_some()))
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let camera = commands.spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: 1.6, ..default() }))).id();
    if std::env::var_os("FPS16_SHOT").is_some() {
        // Render off-screen so screenshots work even when the window is hidden behind others.
        use bevy::render::render_resource::TextureFormat;
        let target = images.add(Image::new_target_texture(1600, 900, TextureFormat::Rgba8UnormSrgb, None));
        commands.entity(camera).insert((bevy::camera::RenderTarget::from(target.clone()), IsDefaultUiCamera));
        commands.insert_resource(ShotTarget(target));
    }
    commands.insert_resource(Ids::new(camera));
    // Drawn last, so on top of the game's HUD.
    commands.spawn((
        StatusText,
        Text::new(""),
        TextLayout { justify: Justify::Center, ..default() },
        GlobalZIndex(10),
        Node { position_type: PositionType::Absolute, top: percent(45), width: percent(100), justify_content: JustifyContent::Center, ..default() },
    ));
    commands.spawn((
        NetGraphText,
        Text::new(""),
        TextFont { font_size: FontSize::Px(13.0), ..default() },
        GlobalZIndex(10),
        Node { position_type: PositionType::Absolute, left: px(12), top: px(12), ..default() },
        Visibility::Hidden,
    ));
}

fn start(mut client: ResMut<Client>, mut renderer: Renderer) {
    let cmds = client.game.start();
    renderer.apply(cmds);
}

/// Every frame rather than every tick, so snapshots aren't left waiting in the socket.
fn receive(mut net: ResMut<Net>, mut client: ResMut<Client>) {
    let Client { game, netcode, incompatible, wrong_build } = &mut *client;
    for packet in net.recv() {
        match decode::<ServerMsg>(&packet) {
            Some(ServerMsg::Snapshot(snap)) => {
                let Some(snap) = netcode.snapshot(snap, &mut |e, i, s| game.predict(e, i, s, false)) else { continue };
                let snap = ClientSnapshot { tick: snap.tick, you: snap.you, state: snap.state.clone(), entities: snap.entities.clone(), events: snap.events.clone() };
                game.snapshot(&snap);
            }
            Some(ServerMsg::WrongBuild { server }) => *wrong_build = Some(server),
            None => *incompatible = true,
        }
    }
}

/// Keys held now, plus the synthetic ones the debug switches press.
fn held_keys(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> Vec<Key> {
    let mut held: Vec<Key> = KEYS.iter().filter(|(code, _)| keys.pressed(*code)).map(|(_, k)| *k).collect();
    held.extend(BUTTONS.iter().filter(|(b, _)| mouse.pressed(*b)).map(|(_, k)| *k));
    // FPS16_FIRE clicks repeatedly, for screenshots of effects; FPS16_STRAFE runs left and right, for checking
    // prediction without a person at the keys.
    if std::env::var_os("FPS16_FIRE").is_some() && clock() % 0.125 < 0.0625 {
        held.push(Key::MouseLeft);
    }
    if std::env::var_os("FPS16_STRAFE").is_some() {
        held.push(if (clock() * 3.0).sin() > 0.0 { Key::D } else { Key::A });
    }
    held
}

fn captured(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None || std::env::var_os("FPS16_FIRE").is_some()
}

fn net_tick(
    mut net: ResMut<Net>,
    mut client: ResMut<Client>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
) {
    let raw = RawInput { held: held_keys(&keys, &mouse), captured: captured(&cursor), ..default() };
    let Client { game, netcode, .. } = &mut *client;
    let input = game.input(&raw);
    netcode.input(input, &mut |e, i, s| game.predict(e, i, s, true));
    if let Some(packet) = netcode.packet(player_name()) {
        net.send(encode(&packet));
    }
}

#[allow(clippy::too_many_arguments)]
fn frame(
    mut client: ResMut<Client>,
    mut renderer: Renderer,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut clicked: ResMut<Clicked>,
    camera: Single<(&mut Transform, &mut Projection), With<Camera3d>>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    mut gizmos: Gizmos,
) {
    let captured = captured(&cursor);
    let mut pressed: Vec<Key> = KEYS.iter().filter(|(code, _)| keys.just_pressed(*code)).map(|(_, k)| *k).collect();
    pressed.extend(BUTTONS.iter().filter(|(b, _)| mouse.just_pressed(*b)).map(|(_, k)| *k));
    let input = RawInput { held: held_keys(&keys, &mouse), pressed, mouse: if captured { motion.delta } else { Vec2::ZERO }, captured };
    let Client { game, netcode, .. } = &mut *client;
    let f = FrameIn {
        dt: time.delta_secs(),
        time: clock(),
        me: netcode.pred.clone(),
        me_pos: netcode.render_pos(fixed.overstep_fraction(), time.delta_secs()),
        others: netcode.interpolate(),
        input,
        clicked: clicked.0.take(),
    };
    let out = game.frame(&f);
    renderer.apply(out.commands);
    renderer.play(&out.sounds);
    let (mut cam, mut projection) = camera.into_inner();
    (cam.translation, cam.rotation) = (out.camera.pos, out.camera.rot);
    if let Projection::Perspective(p) = &mut *projection {
        p.fov = out.camera.fov;
    }
    for (a, b, c) in out.lines {
        gizmos.line(a, b, color(c));
    }
    for (at, radius, c) in out.spheres {
        gizmos.sphere(Isometry3d::from_translation(at), radius, color(c));
    }
}

fn grab_cursor(mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>, mouse: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    // Always the way out, whatever the game does.
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn overlays(
    mut client: ResMut<Client>,
    mut status: Single<&mut Text, (With<StatusText>, Without<NetGraphText>)>,
    graph: Single<(&mut Text, &mut Visibility), With<NetGraphText>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut stats: Local<FrameStats>,
) {
    let c = &mut *client;
    status.0 = match (c.incompatible, c.netcode.me, &c.wrong_build) {
        (_, None, Some(server)) => format!("SERVER IS ON BUILD {server}, YOU HAVE BUILD {BUILD} - restart the game to update"),
        (true, None, _) => "SERVER IS A DIFFERENT VERSION - update both".into(),
        (_, None, _) => "connecting...".into(),
        _ => String::new(),
    };
    let (mut text, mut vis) = graph.into_inner();
    if keys.just_pressed(KeyCode::KeyN) {
        *vis = if *vis == Visibility::Hidden { Visibility::Inherited } else { Visibility::Hidden };
    }
    stats.frames += 1;
    stats.worst_frame_ms = stats.worst_frame_ms.max(time.delta_secs() * 1000.0);
    if stats.since.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
        let (ping, interp) = (c.netcode.ping_ms, c.netcode.interp_ticks / TICK_HZ * 1000.0);
        let net = &mut c.netcode.graph;
        text.0 = format!(
            "fps {} (worst frame {:.0} ms)   ping {ping:.0} ms   worst snapshot gap {:.0} ms   correction {:.0} cm   interp {interp:.0} ms, starved {} frames",
            stats.frames, stats.worst_frame_ms, net.max_gap_ms, net.max_fix_cm, net.starved
        );
        if std::env::var_os("FPS16_NETLOG").is_some() {
            println!("{}", text.0);
        }
        (net.max_gap_ms, net.max_fix_cm, net.starved) = (0.0, 0.0, 0);
        *stats = FrameStats { since: Some(Instant::now()), ..default() };
    }
}

/// The logged-in macOS user (also set for apps launched from Finder).
fn player_name() -> String {
    static NAME: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "player".into()));
    NAME.clone()
}

/// For checking visuals without a person at the screen: `FPS16_SHOT=out.png` saves one screenshot after
/// `FPS16_SHOT_AT` seconds (default 4) and quits; `FPS16_POSE="x,y,z,yaw,pitch"` places the camera (degrees);
/// `FPS16_FIRE` keeps clicking fire, `FPS16_STRAFE` runs left and right.
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
            cam.rotation = sdk::client::look_rot(yaw.to_radians(), pitch.to_radians());
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

const BUTTONS: [(MouseButton, Key); 3] = [(MouseButton::Left, Key::MouseLeft), (MouseButton::Right, Key::MouseRight), (MouseButton::Middle, Key::MouseMiddle)];

#[rustfmt::skip]
const KEYS: [(KeyCode, Key); 51] = [
    (KeyCode::KeyA, Key::A), (KeyCode::KeyB, Key::B), (KeyCode::KeyC, Key::C), (KeyCode::KeyD, Key::D), (KeyCode::KeyE, Key::E),
    (KeyCode::KeyF, Key::F), (KeyCode::KeyG, Key::G), (KeyCode::KeyH, Key::H), (KeyCode::KeyI, Key::I), (KeyCode::KeyJ, Key::J),
    (KeyCode::KeyK, Key::K), (KeyCode::KeyL, Key::L), (KeyCode::KeyM, Key::M), (KeyCode::KeyN, Key::N), (KeyCode::KeyO, Key::O),
    (KeyCode::KeyP, Key::P), (KeyCode::KeyQ, Key::Q), (KeyCode::KeyR, Key::R), (KeyCode::KeyS, Key::S), (KeyCode::KeyT, Key::T),
    (KeyCode::KeyU, Key::U), (KeyCode::KeyV, Key::V), (KeyCode::KeyW, Key::W), (KeyCode::KeyX, Key::X), (KeyCode::KeyY, Key::Y),
    (KeyCode::KeyZ, Key::Z),
    (KeyCode::Digit0, Key::Digit0), (KeyCode::Digit1, Key::Digit1), (KeyCode::Digit2, Key::Digit2), (KeyCode::Digit3, Key::Digit3),
    (KeyCode::Digit4, Key::Digit4), (KeyCode::Digit5, Key::Digit5), (KeyCode::Digit6, Key::Digit6), (KeyCode::Digit7, Key::Digit7),
    (KeyCode::Digit8, Key::Digit8), (KeyCode::Digit9, Key::Digit9),
    (KeyCode::Space, Key::Space), (KeyCode::Enter, Key::Enter), (KeyCode::Escape, Key::Escape), (KeyCode::Tab, Key::Tab),
    (KeyCode::Backspace, Key::Backspace), (KeyCode::ShiftLeft, Key::ShiftLeft), (KeyCode::ShiftRight, Key::ShiftRight),
    (KeyCode::ControlLeft, Key::ControlLeft), (KeyCode::ControlRight, Key::ControlRight), (KeyCode::AltLeft, Key::AltLeft),
    (KeyCode::AltRight, Key::AltRight),
    (KeyCode::ArrowUp, Key::Up), (KeyCode::ArrowDown, Key::Down), (KeyCode::ArrowLeft, Key::Left), (KeyCode::ArrowRight, Key::Right),
];

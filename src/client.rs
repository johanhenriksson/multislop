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
pub struct Net {
    sock: UdpSocket,
    /// The server we play on; None while browsing for one.
    pub server: Option<SocketAddr>,
    lag: Duration,
    /// Extra random delay on packets both ways, kept in order, to mimic bursty Wi-Fi.
    jitter: Duration,
    rng: u32,
    out_q: VecDeque<(Instant, Vec<u8>)>,
    in_q: VecDeque<(Instant, SocketAddr, Vec<u8>)>,
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
        let last = if outgoing { self.out_q.back().map(|q| q.0) } else { self.in_q.back().map(|q| q.0) };
        last.map_or(at, |last| at.max(last))
    }

    fn flush(&mut self) {
        while self.out_q.front().is_some_and(|(t, _)| *t <= Instant::now()) {
            let (_, data) = self.out_q.pop_front().unwrap();
            if let Some(server) = self.server {
                let _ = self.sock.send_to(&data, server);
            }
        }
    }

    /// Packets due, with who sent them.
    fn recv(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        let mut buf = [0u8; 65536];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            let at = self.fake_arrival(false);
            self.in_q.push_back((at, from, buf[..n].to_vec()));
        }
        let mut out = vec![];
        while self.in_q.front().is_some_and(|(t, ..)| *t <= Instant::now()) {
            let (_, from, data) = self.in_q.pop_front().unwrap();
            out.push((from, data));
        }
        out
    }

    /// Forgets the server and anything still queued to or from it.
    fn leave(&mut self) {
        self.server = None;
        self.out_q.clear();
        self.in_q.clear();
    }

    /// Sends to every server port on the LAN (and this machine), skipping the fake lag.
    pub fn broadcast(&self, data: &[u8]) {
        for port in DISCOVERY_PORTS {
            let _ = self.sock.send_to(data, (std::net::Ipv4Addr::BROADCAST, port));
        }
    }
}

/// Biggest game module we'll download.
const MAX_MODULE: u32 = 64 << 20;

/// Getting the game the server runs.
enum Fetch {
    /// Asking the server which game it runs.
    Asking,
    Downloading { hash: [u8; 32], cookie: u64, data: Vec<u8>, have: Vec<bool>, left: usize, cursor: usize },
    Done,
}

#[derive(Resource)]
struct Client {
    /// Loaded once the server has said which game it runs (or from `--game`).
    game: Option<Game>,
    /// `start` has been called and applied.
    started: bool,
    /// The game module's bytes, for a fresh instance when the map changes.
    module: Vec<u8>,
    /// The map the world is built for, and the one the server says it's on.
    map: Option<String>,
    want_map: Option<String>,
    /// The newest snapshot that came before the world for its map was built, handed to the game once it is.
    deferred: Option<ClientSnapshot>,
    fetch: Fetch,
    /// Which of the server's games we're in (`ServerMsg::Game::game`).
    generation: u8,
    /// Why the game couldn't be loaded.
    error: Option<String>,
    /// When we first asked the server which game it runs (clock).
    asked_at: Option<f64>,
    /// When we last heard from the server (clock).
    heard_at: f64,
    /// The module from `--game`, run instead of the server's.
    local: Option<Vec<u8>>,
    interp_ms: u64,
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

/// Seconds without a word from the server before giving up on it, in a game and while connecting.
const LOST_SECS: f64 = 5.0;
const NO_ANSWER_SECS: f64 = 10.0;

impl Client {
    fn new(local: Option<Vec<u8>>, interp_ms: u64) -> Client {
        let game = local.as_ref().map(|bytes| -> Game {
            Box::new(crate::wasm::WasmClient::load(bytes).unwrap_or_else(|e| panic!("can't load the game: {e:?}")))
        });
        Client {
            fetch: if game.is_some() { Fetch::Done } else { Fetch::Asking },
            generation: 0,
            game,
            started: false,
            module: local.clone().unwrap_or_default(),
            map: None,
            want_map: None,
            deferred: None,
            error: None,
            asked_at: None,
            heard_at: 0.0,
            local,
            interp_ms,
            netcode: NetClient::new(interp_ms),
            incompatible: false,
            wrong_build: None,
        }
    }
}

/// Joins `server` (or lets the player pick one from the LAN), running the `local` module if given, or else the game
/// the server runs (downloaded if need be). `hostable` are module files the browser can host a LAN game with.
pub fn run(server: Option<SocketAddr>, local: Option<Vec<u8>>, hostable: Vec<std::path::PathBuf>, lag_ms: u64, jitter_ms: u64, interp_ms: u64) {
    let sock = UdpSocket::bind("0.0.0.0:0").expect("bind");
    sock.set_nonblocking(true).unwrap();
    sock.set_broadcast(true).unwrap();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("Multislop build {BUILD}"),
                        present_mode: PresentMode::AutoNoVsync,
                        // Screenshot runs render off-screen: no window popping up or taking the keyboard.
                        visible: std::env::var_os("FPS16_SHOT").is_none(),
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
        .insert_resource(Client::new(local, interp_ms))
        .insert_resource(crate::browser::Browser::new(hostable))
        .insert_resource(Me::load())
        .init_resource::<crate::browser::Paused>()
        .add_systems(Startup, (setup, crate::browser::setup, crate::browser::setup_pause))
        .add_systems(PreUpdate, receive)
        .add_systems(FixedUpdate, net_tick)
        .add_systems(Update, (grab_cursor, crate::browser::pause, watchdog, crate::browser::browse, crate::browser::animate, start, frame, crate::render::anchor_ui, overlays).chain())
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

/// Back to the server browser when the server goes quiet, never answers, runs another build, or we disconnect: the game's world,
/// HUD and sounds go, the netcode starts over, and the mouse is freed.
fn watchdog(
    mut client: ResMut<Client>,
    mut net: ResMut<Net>,
    mut renderer: Renderer,
    mut browser: ResMut<crate::browser::Browser>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut paused: ResMut<crate::browser::Paused>,
) {
    let Some(server) = net.server else { return };
    let now = clock();
    let silent = |secs: f64| now - client.heard_at > secs;
    let why = match &client.wrong_build {
        _ if paused.pick == Some(crate::browser::Pick::Disconnect) => String::new(),
        Some(build) => format!("{server} runs build {build}, you have build {BUILD} - restart the game to update"),
        None if client.started && silent(LOST_SECS) => format!("lost connection to {server}"),
        None if !client.started && client.asked_at.is_some_and(|t| now - t > NO_ANSWER_SECS) && silent(NO_ANSWER_SECS) => {
            format!("no answer from {server}")
        }
        None => return,
    };
    renderer.clear();
    net.leave();
    *client = Client::new(client.local.take(), client.interp_ms);
    browser.show_error(why);
    *paused = default();
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

/// Builds the game's world and HUD once it's loaded and a server is picked.
fn start(mut client: ResMut<Client>, mut renderer: Renderer, net: Res<Net>) {
    let Some(want) = client.want_map.clone() else { return };
    if net.server.is_none() || client.game.is_none() || client.map.as_ref() == Some(&want) {
        return;
    }
    if client.map.is_some() {
        // Another map: a fresh instance of the game, and its world from scratch.
        renderer.clear();
        match crate::wasm::WasmClient::load(&client.module) {
            Ok(game) => client.game = Some(Box::new(game)),
            Err(e) => return client.error = Some(format!("can't restart the game: {e}")),
        }
    }
    let deferred = client.deferred.take();
    let Some(game) = &mut client.game else { return };
    let cmds = game.start(&want);
    renderer.apply(cmds);
    if let Some(snap) = deferred {
        game.snapshot(&snap);
    }
    (client.map, client.started) = (Some(want), true);
}

/// Most downloaded (or compiled) games kept; the least recently used go first.
const KEEP_CACHED: usize = 4;

pub fn data_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join("Library/Application Support/fps16")
}

/// Where downloaded game modules are kept.
pub fn games_dir() -> std::path::PathBuf {
    data_dir().join("games")
}

/// Marks a cached file as just used, so pruning keeps it.
pub fn touch(path: &std::path::Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Deletes all but the `KEEP_CACHED` most recently used `.ext` files in `dir`.
pub fn prune(dir: &std::path::Path, ext: &str) {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, old) in files.into_iter().skip(KEEP_CACHED) {
        let _ = std::fs::remove_file(old);
    }
}

/// Keeps a module in the download cache (if it isn't already), so joining a server running it needs no download.
pub fn store(module: &[u8]) {
    let path = cache_path(&sha256(module));
    if path.exists() {
        return touch(&path);
    }
    match path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, module)) {
        Ok(()) => prune(&games_dir(), "wasm"),
        Err(e) => eprintln!("can't cache the game at {}: {e}", path.display()),
    }
}

fn cache_path(hash: &[u8; 32]) -> std::path::PathBuf {
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    games_dir().join(format!("{hex}.wasm"))
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

/// Loads a downloaded or cached module as the game.
fn load(client: &mut Client, bytes: &[u8]) {
    match crate::wasm::WasmClient::load(bytes) {
        Ok(game) => {
            println!("loaded the server's game ({} KB)", bytes.len() / 1024);
            client.game = Some(Box::new(game));
            client.module = bytes.to_vec();
        }
        Err(e) => client.error = Some(format!("can't load the server's game: {e}")),
    }
    client.fetch = Fetch::Done;
}

/// The server's answer to which game it runs: cached already, or to download.
fn on_game(client: &mut Client, hash: [u8; 32], size: u32, cookie: u64, generation: u8) {
    client.generation = generation;
    if let Some(bytes) = std::fs::read(cache_path(&hash)).ok().filter(|b| sha256(b) == hash) {
        touch(&cache_path(&hash));
        return load(client, &bytes);
    }
    if size > MAX_MODULE {
        client.error = Some(format!("the server's game is too big ({} MB)", size >> 20));
        client.fetch = Fetch::Done;
        return;
    }
    let chunks = (size as usize).div_ceil(CHUNK);
    client.fetch = Fetch::Downloading { hash, cookie, data: vec![0; size as usize], have: vec![false; chunks], left: chunks, cursor: 0 };
}

fn on_chunk(client: &mut Client, index: u32, chunk: &[u8]) {
    let Fetch::Downloading { hash, data, have, left, .. } = &mut client.fetch else { return };
    let i = index as usize;
    let at = i * CHUNK;
    if have.get(i) != Some(&false) || at + chunk.len() > data.len() {
        return;
    }
    data[at..at + chunk.len()].copy_from_slice(chunk);
    (have[i], *left) = (true, *left - 1);
    if *left > 0 {
        return;
    }
    if sha256(data) != *hash {
        // Corrupt or the server changed games: start over.
        client.fetch = Fetch::Asking;
        return;
    }
    let data = std::mem::take(data);
    store(&data);
    load(client, &data);
}

/// Every frame rather than every tick, so snapshots aren't left waiting in the socket.
fn receive(mut net: ResMut<Net>, mut client: ResMut<Client>, mut browser: ResMut<crate::browser::Browser>, mut renderer: Renderer) {
    for (from, packet) in net.recv() {
        if Some(from) != net.server {
            if net.server.is_none() {
                browser.reply(from, decode(&packet));
            }
            continue;
        }
        let msg = decode::<ServerMsg>(&packet);
        if msg.is_some() {
            client.heard_at = clock();
        }
        match msg {
            Some(ServerMsg::Game { hash, size, cookie, game }) if matches!(client.fetch, Fetch::Asking) => on_game(&mut client, hash, size, cookie, game),
            // The server moved on to another game: start over with it, as if we had just connected.
            Some(ServerMsg::Game { hash, size, cookie, game }) if game != client.generation && matches!(client.fetch, Fetch::Done) => {
                renderer.clear();
                *client = Client { heard_at: clock(), ..Client::new(None, client.interp_ms) };
                on_game(&mut client, hash, size, cookie, game);
            }
            Some(ServerMsg::Chunk { index, data }) => on_chunk(&mut client, index, &data),
            Some(ServerMsg::Game { .. } | ServerMsg::Info { .. }) => {}
            // From the game before, overtaken on the way by the news of the next one.
            Some(ServerMsg::Snapshot(delta)) if delta.game != client.generation => {}
            Some(ServerMsg::Snapshot(delta)) => {
                let base = delta.base.and_then(|t| client.netcode.snapshot_at(t));
                // A base we no longer have: skip it; the server falls back to whole snapshots soon enough.
                let Some(snap) = apply(base, delta) else { continue };
                let Client { game: Some(game), netcode, map, want_map, deferred, .. } = &mut *client else { continue };
                let Some(snap) = netcode.snapshot(snap, &mut |e, i, s| game.predict(e, i, s, false)) else { continue };
                let view = ClientSnapshot { tick: snap.tick, you: snap.you, state: snap.state.clone(), entities: snap.entities.clone(), events: snap.events.clone() };
                // The world isn't built for this map yet: `start` does that first, then gets this snapshot.
                if map.as_ref() != Some(&snap.map) {
                    (*want_map, *deferred) = (Some(snap.map.clone()), Some(view));
                    continue;
                }
                game.snapshot(&view);
            }
            Some(ServerMsg::WrongBuild { server }) => client.wrong_build = Some(server),
            None => client.incompatible = true,
        }
    }
}

/// Controller buttons as the keys games already read, so every game plays with a controller unchanged: the left
/// stick is WASD (`stick_keys`) and the right stick the mouse (`stick_look`).
const PAD: [(GamepadButton, Key); 14] = [
    (GamepadButton::South, Key::Space),
    (GamepadButton::East, Key::ControlLeft),
    (GamepadButton::West, Key::R),
    (GamepadButton::North, Key::Q),
    (GamepadButton::RightTrigger2, Key::MouseLeft),
    (GamepadButton::LeftTrigger2, Key::MouseRight),
    (GamepadButton::RightTrigger, Key::E),
    (GamepadButton::LeftTrigger, Key::G),
    (GamepadButton::LeftThumb, Key::ShiftLeft),
    (GamepadButton::DPadUp, Key::Digit1),
    (GamepadButton::DPadRight, Key::Digit2),
    (GamepadButton::DPadDown, Key::Digit3),
    (GamepadButton::DPadLeft, Key::Digit4),
    (GamepadButton::Select, Key::Tab),
];
/// Start opens the buy menu, where the d-pad picks.
const PAD_START: Key = Key::B;
/// Look speed at full right stick, in mouse counts per second; the stick is squared for fine aim near the middle.
const PAD_LOOK: f32 = 1800.0;

fn pad_key(button: GamepadButton) -> Option<Key> {
    if button == GamepadButton::Start { Some(PAD_START) } else { PAD.iter().find(|(b, _)| *b == button).map(|(_, k)| *k) }
}

/// The left stick as WASD, past half way on each axis.
fn stick_keys(pad: &Gamepad) -> impl Iterator<Item = Key> {
    let s = pad.left_stick();
    [(s.y > 0.5, Key::W), (s.y < -0.5, Key::S), (s.x > 0.5, Key::D), (s.x < -0.5, Key::A)].into_iter().filter(|(on, _)| *on).map(|(_, k)| k)
}

/// The right stick as mouse motion over `dt` seconds.
fn stick_look(pads: &Query<&Gamepad>, dt: f32) -> Vec2 {
    let s: Vec2 = pads.iter().map(|p| p.right_stick()).sum();
    Vec2::new(s.x, -s.y) * s.length().min(1.0) * PAD_LOOK * dt
}

/// Keys held now, from the keyboard, mouse and controllers, plus the synthetic ones the debug switches press.
fn held_keys(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>, pads: &Query<&Gamepad>) -> Vec<Key> {
    let mut held: Vec<Key> = KEYS.iter().filter(|(code, _)| keys.pressed(*code)).map(|(_, k)| *k).collect();
    held.extend(BUTTONS.iter().filter(|(b, _)| mouse.pressed(*b)).map(|(_, k)| *k));
    for pad in pads {
        held.extend(pad.get_pressed().filter_map(|b| pad_key(*b)).chain(stick_keys(pad)));
    }
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
    me: Res<Me>,
    paused: Res<crate::browser::Paused>,
    mut net: ResMut<Net>,
    mut client: ResMut<Client>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<&Gamepad>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut ticks: Local<u32>,
) {
    *ticks += 1;
    if net.server.is_none() {
        return;
    }
    let mut send = |msg| net.send(encode(&ClientPacket { build: BUILD.into(), msg }));
    client.asked_at.get_or_insert(clock());
    match &mut client.fetch {
        Fetch::Asking if *ticks % 64 == 1 => send(ClientMsg::Hello { pad: vec![0; HELLO_SIZE] }),
        Fetch::Asking => {}
        // Ask for the next missing chunks every tick; lost ones come round again when the cursor wraps.
        Fetch::Downloading { cookie, have, cursor, .. } => {
            let chunks: Vec<u32> = (0..have.len())
                .map(|i| (*cursor + i) % have.len())
                .filter(|&i| !have[i])
                .take(CHUNKS_PER_REQUEST)
                .map(|i| i as u32)
                .collect();
            *cursor = chunks.last().map_or(0, |&i| i as usize + 1);
            send(ClientMsg::Chunks { cookie: *cookie, chunks });
        }
        Fetch::Done => {
            let raw = if paused.open { default() } else { RawInput { held: held_keys(&keys, &mouse, &pads), captured: captured(&cursor), ..default() } };
            let Client { game: Some(game), netcode, generation, .. } = &mut *client else { return };
            let input = game.input(&raw);
            netcode.input(input, &mut |e, i, s| game.predict(e, i, s, true));
            if let Some(packet) = netcode.packet(me.0.clone(), *generation) {
                send(ClientMsg::Input(packet));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn frame(
    mut client: ResMut<Client>,
    paused: Res<crate::browser::Paused>,
    mut renderer: Renderer,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    pads: Query<&Gamepad>,
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
    pressed.extend(pads.iter().flat_map(|p| p.get_just_pressed().filter_map(|b| pad_key(*b))));
    let look = motion.delta + stick_look(&pads, time.delta_secs());
    let input = if paused.open { default() } else { RawInput { held: held_keys(&keys, &mouse, &pads), pressed, mouse: if captured { look } else { Vec2::ZERO }, captured } };
    if !client.started {
        return;
    }
    let Client { game: Some(game), netcode, .. } = &mut *client else { return };
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

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    client: Res<Client>,
    net: Res<Net>,
    paused: Res<crate::browser::Paused>,
) {
    // Not while picking a server, loading or in the Esc menu: the mouse is for the menus then.
    let pad = pads.iter().any(|p| p.get_just_pressed().next().is_some());
    if (mouse.just_pressed(MouseButton::Left) || pad) && client.started && net.server.is_some() && !paused.open {
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
    net: Res<Net>,
    mut status: Single<&mut Text, (With<StatusText>, Without<NetGraphText>)>,
    graph: Single<(&mut Text, &mut Visibility), With<NetGraphText>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut stats: Local<FrameStats>,
) {
    let c = &mut *client;
    status.0 = match (c.incompatible, c.netcode.me, &c.wrong_build, &c.fetch) {
        _ if net.server.is_none() => String::new(),
        (_, None, Some(server), _) => format!("SERVER IS ON BUILD {server}, YOU HAVE BUILD {BUILD} - restart the game to update"),
        (true, None, ..) => "SERVER IS A DIFFERENT VERSION - update both".into(),
        _ if c.error.is_some() => c.error.clone().unwrap_or_default(),
        (_, None, _, Fetch::Downloading { have, left, .. }) => format!("downloading the game... {}%", 100 - 100 * left / have.len().max(1)),
        (_, None, _, Fetch::Asking) if c.asked_at.is_some_and(|t| clock() - t > 3.0) => format!(
            "no answer from {} - is a server running there, on build {BUILD}?",
            net.server.map_or(String::new(), |a| a.to_string())
        ),
        (_, None, ..) => "connecting...".into(),
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

/// Who we are in every game: the logged-in macOS user (also set for apps launched from Finder). Games give everyone a
/// color of their own.
#[derive(Resource)]
pub struct Me(pub sdk::Profile);

impl Me {
    pub fn load() -> Me {
        let name = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "player".into());
        Me(sdk::Profile { name })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_takes_chunks_in_any_order_and_restarts_on_a_bad_hash() {
        let mut client = Client::new(None, 50);
        let module: Vec<u8> = (0..3000).map(|i| i as u8).collect();
        // The server claims a different hash than the bytes it sends, so nothing gets cached or loaded.
        on_game(&mut client, sha256(b"something else"), module.len() as u32, 7, 0);
        let Fetch::Downloading { cookie: 7, ref have, .. } = client.fetch else { panic!("should be downloading") };
        assert_eq!(have.len(), 3);
        for i in [2, 0, 0, 2] {
            on_chunk(&mut client, i, &module.chunks(CHUNK).nth(i as usize).unwrap());
        }
        let Fetch::Downloading { left: 1, ref data, .. } = client.fetch else { panic!("one chunk left") };
        assert_eq!(&data[..CHUNK], &module[..CHUNK]);
        on_chunk(&mut client, 9, &[1, 2, 3]);
        on_chunk(&mut client, 1, &module[CHUNK..2 * CHUNK]);
        assert!(matches!(client.fetch, Fetch::Asking) && client.game.is_none(), "hash mismatch: ask again");
    }
}

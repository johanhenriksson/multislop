//! The LAN server browser, shown until a server is picked: asks every server port on the network who's there each
//! second and lists the answers (name, map, players, ping). Click one, pick with the arrow keys and Enter, type an
//! address and press Enter, or host a LAN game from here for others to join.

use crate::client::Net;
use crate::net::*;
use crate::netclient::clock;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use sdk::{GameInfo, encode};
use std::collections::BTreeMap;
use std::net::{SocketAddr, ToSocketAddrs};

/// Where the last typed address is kept (the launcher used to ask for it there too).
fn last_address_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join("Library/Application Support/fps16/host")
}

/// `host` or `host:port`, port 5000 by default.
pub fn resolve(text: &str) -> Option<SocketAddr> {
    let text = text.trim();
    let with_port = if text.contains(':') { text.to_string() } else { format!("{text}:5000") };
    with_port.to_socket_addrs().ok()?.next()
}

struct Found {
    name: String,
    /// Its build when it isn't ours (then there's no game info).
    build: Option<String>,
    game: Option<GameInfo>,
    ping_ms: f32,
    /// When we last heard from it (clock).
    seen: f64,
}

#[derive(Resource)]
pub struct Browser {
    found: BTreeMap<SocketAddr, Found>,
    /// Typed address.
    address: String,
    selected: usize,
    /// When we last asked who's there.
    asked: f64,
    /// A clicked server, joined on the next frame.
    clicked: Option<SocketAddr>,
    /// Module files "Host a LAN game" offers.
    hostable: Vec<std::path::PathBuf>,
    host_clicked: bool,
    /// The games found in `hostable` when the host button was clicked: label and module.
    games: Vec<(String, Vec<u8>)>,
    /// A game picked to host, by index into `games`.
    host_pick: Option<usize>,
    error: String,
    /// The list changed and its rows need rebuilding.
    dirty: bool,
}

impl Browser {
    pub fn new(hostable: Vec<std::path::PathBuf>) -> Browser {
        let address = std::fs::read_to_string(last_address_path()).unwrap_or_default().trim().to_string();
        Browser {
            found: BTreeMap::new(),
            address,
            selected: 0,
            asked: f64::MIN,
            clicked: None,
            hostable,
            host_clicked: false,
            games: Vec::new(),
            host_pick: None,
            error: String::new(),
            dirty: true,
        }
    }
}

/// Column widths in pixels: name, map, players, ping.
const COLUMNS: [f32; 4] = [330.0, 130.0, 170.0, 80.0];

/// A row of cells in the column widths.
fn row(commands: &mut Commands, parent: Entity, cells: [String; 4], size: f32, color: Color) -> Entity {
    let row = commands.spawn((ChildOf(parent), Node { padding: UiRect::axes(px(10), px(6)), ..default() })).id();
    for (cell, width) in cells.into_iter().zip(COLUMNS) {
        let font = TextFont { font_size: FontSize::Px(size), ..default() };
        commands.spawn((ChildOf(row), Node { width: px(width), overflow: Overflow::clip(), ..default() }, Text::new(cell), font, TextColor(color)));
    }
    row
}

impl Browser {
    /// Joinable servers first, then by name.
    fn sorted(&self) -> Vec<(SocketAddr, &Found)> {
        let mut rows: Vec<(SocketAddr, &Found)> = self.found.iter().map(|(a, f)| (*a, f)).collect();
        rows.sort_by_key(|(_, f)| (f.game.is_none(), f.name.clone()));
        rows
    }

    /// An answer to our broadcast.
    pub fn reply(&mut self, from: SocketAddr, msg: Option<ServerMsg>) {
        let (name, build, game, ping_ms) = match msg {
            Some(ServerMsg::Info { name, game, time }) => (name, None, Some(game), ((clock() - time) * 1000.0) as f32),
            Some(ServerMsg::WrongBuild { server }) => (format!("{from}  (other version)"), Some(server), None, 0.0),
            _ => return,
        };
        let changed = self.found.get(&from).is_none_or(|f| f.name != name || f.game != game || (f.ping_ms - ping_ms).abs() > 2.0);
        self.found.insert(from, Found { name, build, game, ping_ms, seen: clock() });
        self.dirty |= changed;
    }
}

#[derive(Component)]
pub struct Panel;

#[derive(Component)]
pub struct List;

#[derive(Component)]
pub struct AddressText;

#[derive(Component)]
pub struct ErrorText;

#[derive(Component)]
pub struct HostButton;

/// The games to pick from, under the host button.
#[derive(Component)]
pub struct HostList;

/// Loads each module once (identical files only once) and asks it what it is.
fn find_games(paths: &[std::path::PathBuf]) -> Vec<(String, Vec<u8>)> {
    let mut seen = std::collections::HashSet::new();
    let mut games = Vec::new();
    for path in paths {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if !seen.insert(crate::client::sha256(&bytes)) {
            continue;
        }
        let Ok(mut game) = crate::wasm::WasmGame::load(&bytes) else { continue };
        let info = sdk::ServerGame::info(&mut game);
        let file: String = path.file_stem().map(|f| f.to_string_lossy().chars().take(12).collect()).unwrap_or_default();
        let game = if info.game.is_empty() { "Unnamed game".into() } else { info.game };
        games.push((format!("{game} on {}  ({file})", info.map), bytes));
    }
    games
}

pub fn setup(mut commands: Commands) {
    let text = |size: f32, color: Color| (TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color));
    let gray = Color::srgb(0.6, 0.6, 0.6);
    commands.spawn((
        Panel,
        Visibility::Hidden,
        GlobalZIndex(5),
        BackgroundColor(Color::srgb(0.08, 0.08, 0.1)),
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, ..default() },
        children![(
            Node { flex_direction: FlexDirection::Column, row_gap: px(14), margin: UiRect::top(percent(10)), width: px(760), ..default() },
            children![
                (Text::new("Servers on your network"), text(30.0, Color::srgb(1.0, 0.68, 0.12))),
                (List, Node { flex_direction: FlexDirection::Column, row_gap: px(4), min_height: px(200), ..default() }),
                (AddressText, Text::new(""), text(20.0, Color::WHITE)),
                (ErrorText, Text::new(""), text(16.0, Color::srgb(1.0, 0.4, 0.3))),
                (Text::new("Click a server, or pick one with the arrow keys and press Enter. Or type an address and press Enter."), text(14.0, gray)),
            ],
        )],
    ));
    let amber = Color::srgb(1.0, 0.68, 0.12);
    commands
        .spawn((
            HostButton,
            Visibility::Hidden,
            GlobalZIndex(6),
            Node { position_type: PositionType::Absolute, right: percent(5), top: percent(10), padding: UiRect::axes(px(18), px(10)), border: UiRect::all(px(2)), ..default() },
            BorderColor::all(amber),
            BackgroundColor(Color::srgba(1.0, 0.68, 0.12, 0.12)),
            children![(Text::new("Host a LAN game"), text(20.0, amber))],
        ))
        .observe(|_: On<PointerClick>, mut b: ResMut<Browser>| b.host_clicked = true);
    commands.spawn((
        HostList,
        GlobalZIndex(6),
        Node { position_type: PositionType::Absolute, right: percent(5), top: percent(17), flex_direction: FlexDirection::Column, row_gap: px(4), ..default() },
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn browse(
    mut commands: Commands,
    mut net: ResMut<Net>,
    mut b: ResMut<Browser>,
    mut typed: MessageReader<KeyboardInput>,
    mut panel: Single<&mut Visibility, With<Panel>>,
    list: Single<Entity, With<List>>,
    mut address: Single<&mut Text, (With<AddressText>, Without<ErrorText>)>,
    mut error: Single<&mut Text, With<ErrorText>>,
    mut host_button: Single<&mut Visibility, (With<HostButton>, Without<Panel>)>,
    host_list: Single<Entity, With<HostList>>,
) {
    let browsing = net.server.is_none();
    **host_button = if browsing { Visibility::Inherited } else { Visibility::Hidden };
    if !browsing {
        **panel = Visibility::Hidden;
        typed.clear();
        return;
    }
    **panel = Visibility::Inherited;
    let now = clock();
    if now - b.asked > 1.0 {
        b.asked = now;
        net.broadcast(&encode(&ClientPacket { build: BUILD.into(), msg: ClientMsg::Info { time: now, pad: vec![0; INFO_QUERY_SIZE] } }));
    }
    let before = b.found.len();
    b.found.retain(|_, f| now - f.seen < 3.5);
    b.dirty |= b.found.len() != before;

    let mut join = b.clicked.take();
    if std::mem::take(&mut b.host_clicked) {
        b.games = find_games(&b.hostable);
        b.error = if b.games.is_empty() { "no game modules found to host".into() } else { String::new() };
        commands.entity(*host_list).despawn_children();
        for (i, (label, _)) in b.games.iter().enumerate() {
            commands
                .spawn((
                    ChildOf(*host_list),
                    Node { padding: UiRect::axes(px(12), px(6)), ..default() },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                    children![(Text::new(label.clone()), TextFont { font_size: FontSize::Px(16.0), ..default() })],
                ))
                .observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| b.host_pick = Some(i));
        }
    }
    if let Some((_, module)) = b.host_pick.take().and_then(|i| b.games.get(i).cloned()) {
        let user = std::env::var("USER").unwrap_or_else(|_| "someone".into());
        match crate::server::host(format!("{user}'s game"), module) {
            Ok(port) => join = Some(SocketAddr::from(([127, 0, 0, 1], port))),
            Err(e) => b.error = e,
        }
    }
    for ev in typed.read().filter(|ev| ev.state == ButtonState::Pressed) {
        match &ev.logical_key {
            Key::Character(c) if b.address.len() < 64 => b.address.extend(c.chars().filter(|c| !c.is_control())),
            Key::Backspace => _ = b.address.pop(),
            Key::ArrowUp => (b.selected, b.dirty) = (b.selected.saturating_sub(1), true),
            Key::ArrowDown => (b.selected, b.dirty) = ((b.selected + 1).min(b.found.len().saturating_sub(1)), true),
            Key::Enter if !b.address.trim().is_empty() => match resolve(&b.address) {
                Some(addr) => {
                    let _ = std::fs::write(last_address_path(), format!("{}\n", b.address.trim()));
                    join = Some(addr);
                }
                None => b.error = format!("can't find {}", b.address.trim()),
            },
            Key::Enter => join = b.sorted().get(b.selected).filter(|(_, f)| f.game.is_some()).map(|(a, _)| *a),
            _ => {}
        }
    }
    if let Some(addr) = join {
        net.server = Some(addr);
        commands.entity(*host_list).despawn_children();
        return;
    }
    address.0 = format!("Address: {}_", b.address);
    error.0 = b.error.clone();
    if !b.dirty {
        return;
    }
    b.dirty = false;
    commands.entity(*list).despawn_children();
    let header = ["Name", "Map", "Players", "Ping"].map(String::from);
    row(&mut commands, *list, header, 16.0, Color::srgb(0.6, 0.6, 0.6));
    if b.found.is_empty() {
        commands.spawn((ChildOf(*list), Text::new("Looking for servers..."), TextColor(Color::srgb(0.5, 0.5, 0.5)), Node { margin: UiRect::left(px(10)), ..default() }));
    }
    let selected = b.selected.min(b.found.len().saturating_sub(1));
    for (i, (addr, f)) in b.sorted().into_iter().enumerate() {
        let (cells, color) = match &f.game {
            Some(g) => ([f.name.clone(), g.map.clone(), format!("{} + {} bots", g.players, g.bots), format!("{:.0} ms", f.ping_ms)], Color::WHITE),
            None => {
                let build = format!("build {}", f.build.as_deref().unwrap_or("?"));
                ([f.name.clone(), String::new(), build, String::new()], Color::srgb(0.5, 0.5, 0.5))
            }
        };
        let background = if i == selected { Color::srgba(1.0, 0.68, 0.12, 0.25) } else { Color::srgba(1.0, 1.0, 1.0, 0.04) };
        let r = row(&mut commands, *list, cells, 18.0, color);
        let mut r = commands.entity(r);
        r.insert(BackgroundColor(background));
        if f.game.is_some() {
            r.observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| b.clicked = Some(addr));
        }
    }
}

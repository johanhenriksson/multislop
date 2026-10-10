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
use std::sync::{Arc, Mutex};
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
    /// The games and maps found in `hostable`. Filled in by a background scan.
    games: Arc<Mutex<Option<Vec<Game>>>>,
    /// The game list is open (the host button was clicked), and what it shows, so it's redrawn when that changes.
    host_open: bool,
    shown_games: Option<usize>,
    /// A game picked to host, by index into `games`.
    host_pick: Option<usize>,
    /// Bots for a hosted game; None leaves it to the game.
    bots: Option<u32>,
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
            games: scan(hostable.clone()),
            hostable,
            host_clicked: false,
            host_open: false,
            shown_games: None,
            host_pick: None,
            bots: None,
            error: String::new(),
            dirty: true,
        }
    }
}

/// Most bots the host menu offers.
const MAX_BOTS: u32 = 31;

/// One step through the bot counts the host menu offers: the game's default, then 0 to MAX_BOTS.
fn step_bots(bots: Option<u32>, up: bool) -> Option<u32> {
    match (bots, up) {
        (None, true) => Some(0),
        (None, false) | (Some(0), false) => None,
        (Some(n), true) => Some((n + 1).min(MAX_BOTS)),
        (Some(n), false) => Some(n - 1),
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
    /// Why we're back here (a lost connection, say).
    pub fn show_error(&mut self, why: String) {
        (self.error, self.dirty) = (why, true);
    }

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

/// Something "Host a LAN game" can start: a game module and one of its maps.
#[derive(Clone)]
struct Game {
    label: String,
    module: Vec<u8>,
    map: String,
}

/// Asks each module what it is, keeping only the newest file of each game, and lists every map of each.
fn find_games(paths: &[std::path::PathBuf]) -> Vec<Game> {
    let mut newest: BTreeMap<String, (std::time::SystemTime, Vec<String>, Vec<u8>)> = BTreeMap::new();
    for path in paths {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let modified = path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if newest.values().any(|(at, _, b)| *at >= modified && *b == bytes) {
            continue;
        }
        let Ok(mut game) = crate::wasm::WasmGame::load(&bytes) else { continue };
        let info = sdk::ServerGame::info(&mut game);
        let name = if info.game.is_empty() { "Unnamed game".to_string() } else { info.game };
        if newest.get(&name).is_none_or(|(at, ..)| modified > *at) {
            let maps = if info.maps.is_empty() { vec![info.map] } else { info.maps };
            newest.insert(name, (modified, maps, bytes));
        }
    }
    let games = newest.into_iter().flat_map(|(name, (_, maps, module))| {
        maps.into_iter().map(move |map| Game { label: format!("{name} on {map}"), module: module.clone(), map })
    });
    games.collect()
}

/// Finds the games in `paths` on a thread (each takes a compile the first time), into the returned slot.
fn scan(paths: Vec<std::path::PathBuf>) -> Arc<Mutex<Option<Vec<Game>>>> {
    let slot = Arc::new(Mutex::new(None));
    let out = slot.clone();
    std::thread::spawn(move || {
        let games = find_games(&paths);
        *out.lock().unwrap() = Some(games);
    });
    slot
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
        b.host_open = true;
        // Show what we know now; a fresh look (games downloaded since) replaces it when done.
        let known = b.games.lock().unwrap().clone();
        b.games = scan(b.hostable.clone());
        if known.is_some() {
            *b.games.lock().unwrap() = known;
        }
        b.shown_games = None;
    }
    let games = b.games.lock().unwrap().clone();
    let count = games.as_ref().map(Vec::len);
    if b.host_open && b.shown_games != count.or(Some(usize::MAX)) {
        b.shown_games = count.or(Some(usize::MAX));
        commands.entity(*host_list).despawn_children();
        let row = (Node { padding: UiRect::axes(px(12), px(6)), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)));
        let font = TextFont { font_size: FontSize::Px(16.0), ..default() };
        let label = b.bots.map_or("Bots: game default".to_string(), |n| format!("Bots: {n}"));
        let bots = commands.spawn((ChildOf(*host_list), Node { column_gap: px(14), align_items: AlignItems::Center, ..row.0.clone() }, row.1)).id();
        commands.spawn((ChildOf(bots), Node { width: px(170), ..default() }, Text::new(label), font.clone()));
        for (sign, up) in [("-", false), ("+", true)] {
            let button = Node { padding: UiRect::axes(px(9), px(0)), border: UiRect::all(px(1)), ..default() };
            commands
                .spawn((ChildOf(bots), button, BorderColor::all(Color::srgb(0.6, 0.6, 0.6)), children![(Text::new(sign), font.clone())]))
                .observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| (b.bots, b.shown_games) = (step_bots(b.bots, up), None));
        }
        match &games {
            None => _ = commands.spawn((ChildOf(*host_list), row, children![(Text::new("Finding games..."), font)])),
            Some(list) if list.is_empty() => _ = commands.spawn((ChildOf(*host_list), row, children![(Text::new("No games found to host"), font)])),
            Some(list) => {
                for (i, game) in list.iter().enumerate() {
                    commands
                        .spawn((ChildOf(*host_list), row.clone(), children![(Text::new(game.label.clone()), font.clone())]))
                        .observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| b.host_pick = Some(i));
                }
            }
        }
    }
    if let Some(game) = b.host_pick.take().and_then(|i| games.as_ref()?.get(i).cloned()) {
        let user = std::env::var("USER").unwrap_or_else(|_| "someone".into());
        crate::client::store(&game.module);
        match crate::server::host(format!("{user}'s game"), game.module, game.map, b.bots) {
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
        b.error.clear();
        (b.host_open, b.shown_games) = (false, None);
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

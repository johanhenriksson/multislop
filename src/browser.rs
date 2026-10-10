//! The LAN server browser, shown until a server is picked: asks every server port on the network who's there each
//! second and lists the answers (name, map, players, ping). Click one, pick with the arrow keys and Enter, type an
//! address and press Enter, or host a LAN game from here for others to join.

use crate::client::{Me, Net};
use crate::net::*;
use crate::netclient::clock;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::events::{PointerClick, PointerOut, PointerOver};
use bevy::prelude::*;
use bevy::text::{LetterSpacing, LineHeight};
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
    /// Players for a hosted game, bots filling the rest; None leaves it to the game.
    players: Option<u32>,
    /// Host every game in turn, starting with the picked one.
    rotate: bool,
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
            players: None,
            rotate: false,
            error: String::new(),
            dirty: true,
        }
    }
}

/// Most players the host menu offers.
const MAX_PLAYERS: u32 = 32;

/// One step through the player counts the host menu offers: the game's default, then 1 to MAX_PLAYERS.
fn step_players(players: Option<u32>, up: bool) -> Option<u32> {
    match (players, up) {
        (None, true) => Some(1),
        (None, false) | (Some(1), false) => None,
        (Some(n), true) => Some((n + 1).min(MAX_PLAYERS)),
        (Some(n), false) => Some(n - 1),
    }
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


const INK: Color = Color::srgb(0.94, 0.94, 0.98);
const DIM: Color = Color::srgba(0.85, 0.85, 0.95, 0.5);
const FAINT: Color = Color::srgba(0.85, 0.85, 0.95, 0.28);
const CARD: Color = Color::srgba(1.0, 1.0, 1.0, 0.045);
const CARD_HOT: Color = Color::srgba(1.0, 1.0, 1.0, 0.1);
const LINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.09);
const HOT_PINK: Color = Color::srgb(1.0, 0.24, 0.5);
const HOT_ORANGE: Color = Color::srgb(1.0, 0.6, 0.12);
const BAD: Color = Color::srgb(1.0, 0.35, 0.4);

fn ui_font(size: f32, weight: FontWeight) -> TextFont {
    TextFont { font: FontSource::families("Avenir Next, Helvetica Neue, sans-serif"), font_size: FontSize::Px(size), weight, ..default() }
}

/// The big type: condensed, heavy, slanted.
fn display_font(size: f32) -> TextFont {
    TextFont {
        font: FontSource::families("Futura, Avenir Next, sans-serif"),
        font_size: FontSize::Px(size),
        weight: FontWeight::EXTRA_BOLD,
        width: FontWidth::CONDENSED,
        style: FontStyle::Italic,
        ..default()
    }
}

/// Small spaced-out capitals over a section.
fn label(text: &str) -> impl Bundle {
    (Text::new(text), ui_font(12.0, FontWeight::BOLD), LetterSpacing::Px(4.0), TextColor(DIM), Pickable::IGNORE)
}

fn accent(me: &Me) -> Color {
    let [r, g, b] = me.0.color;
    Color::srgb_u8(r, g, b)
}

/// Lights up under the pointer: `hot` instead of `base`, and a little bigger when `grow`.
#[derive(Component, Clone, Copy)]
struct Hover {
    base: Color,
    hot: Color,
    grow: bool,
}

fn hoverable(mut e: EntityCommands, base: Color, hot: Color, grow: bool) -> EntityCommands {
    e.insert((Hover { base, hot, grow }, BackgroundColor(base), UiTransform::IDENTITY));
    e.observe(|ev: On<PointerOver>, mut q: Query<(&Hover, &mut BackgroundColor, &mut UiTransform)>| {
        if let Ok((h, mut bg, mut t)) = q.get_mut(ev.event_target()) {
            bg.0 = h.hot;
            t.scale = Vec2::splat(if h.grow { 1.04 } else { 1.0 });
        }
    });
    e.observe(|ev: On<PointerOut>, mut q: Query<(&Hover, &mut BackgroundColor, &mut UiTransform)>| {
        if let Ok((h, mut bg, mut t)) = q.get_mut(ev.event_target()) {
            (bg.0, t.scale) = (h.base, Vec2::ONE);
        }
    });
    e
}

/// Server list columns after the name, in pixels: game and map, players, ping.
const COLUMNS: [f32; 3] = [220.0, 150.0, 70.0];

/// A server list row: the name takes what's left. `mark` puts a bar of that color down the left.
fn row(commands: &mut Commands, parent: Entity, cells: [(String, Color); 4], size: f32, mark: Option<Color>) -> Entity {
    let border = UiRect::left(px(if mark.is_some() { 4 } else { 0 }));
    let node = Node { padding: UiRect::axes(px(16), px(12)), border, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(12)), ..default() };
    let row = commands.spawn((ChildOf(parent), node, BorderColor::all(mark.unwrap_or(Color::NONE)))).id();
    for (i, (cell, color)) in cells.into_iter().enumerate() {
        let (width, weight) = if i == 0 { (Val::Auto, FontWeight::SEMIBOLD) } else { (px(COLUMNS[i - 1]), FontWeight::MEDIUM) };
        let node = Node { width, flex_grow: if i == 0 { 1.0 } else { 0.0 }, overflow: Overflow::clip(), ..default() };
        commands.spawn((ChildOf(row), node, Text::new(cell), ui_font(size, weight), TextColor(color), Pickable::IGNORE));
    }
    row
}

#[derive(Component)]
pub struct Panel;

#[derive(Component)]
pub struct List;

#[derive(Component)]
pub struct AddressText;

#[derive(Component)]
pub struct ErrorText;

/// The games to pick from, under the host button.
#[derive(Component)]
pub struct HostList;

/// Lit in the player's color.
#[derive(Component)]
pub struct Title;

#[derive(Component)]
pub struct NameText;

#[derive(Component)]
pub struct NameDot;

#[derive(Component)]
pub struct Swatches;

#[derive(Component)]
pub struct StatusDot;

/// A soft glow drifting around the background; the first is in the player's color.
#[derive(Component)]
pub struct Orb(usize);

/// Glows: color, where (percent of the screen), size in pixels.
const ORBS: [(Color, f32, f32, f32); 4] = [
    (Color::WHITE, -8.0, -15.0, 900.0),
    (HOT_PINK, 55.0, 45.0, 1000.0),
    (Color::srgb(0.2, 0.5, 1.0), 70.0, -30.0, 800.0),
    (HOT_ORANGE, 5.0, 60.0, 700.0),
];

fn glow(color: Color, strength: f32) -> BackgroundGradient {
    let stops = vec![ColorStop::auto(color.with_alpha(strength)), ColorStop::auto(color.with_alpha(strength * 0.3)), ColorStop::auto(color.with_alpha(0.0))];
    RadialGradient::new(UiPosition::CENTER, RadialGradientShape::ClosestSide, stops).into()
}

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

pub fn setup(mut commands: Commands, me: Res<Me>) {
    let you = accent(&me);
    let full = Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), overflow: Overflow::clip(), ..default() };
    let backdrop = LinearGradient::degrees(160.0, vec![
        ColorStop::auto(Color::srgb(0.09, 0.03, 0.16)),
        ColorStop::auto(Color::srgb(0.02, 0.02, 0.05)),
        ColorStop::auto(Color::srgb(0.0, 0.06, 0.09)),
    ]);
    let panel = commands.spawn((Panel, Visibility::Hidden, GlobalZIndex(5), full.clone(), BackgroundColor(Color::BLACK), BackgroundGradient::from(backdrop))).id();
    for (i, (color, x, y, size)) in ORBS.into_iter().enumerate() {
        let color = if i == 0 { you } else { color };
        let node = Node { position_type: PositionType::Absolute, left: percent(x), top: percent(y), width: px(size), height: px(size), ..default() };
        commands.spawn((ChildOf(panel), Orb(i), node, glow(color, 0.22), UiTransform::IDENTITY, Pickable::IGNORE));
    }
    // A slash of light behind the title.
    let slash = Node { position_type: PositionType::Absolute, left: percent(-10), top: percent(8), width: percent(70), height: px(220), ..default() };
    let stops = vec![ColorStop::auto(Color::NONE), ColorStop::auto(Color::srgba(1.0, 1.0, 1.0, 0.035)), ColorStop::auto(Color::NONE)];
    commands.spawn((ChildOf(panel), slash, BackgroundGradient::from(LinearGradient::to_right(stops)), UiTransform::from_rotation(Rot2::degrees(-8.0)), Pickable::IGNORE));

    let content = Node { width: percent(100), height: percent(100), padding: UiRect::axes(percent(6), percent(7)), column_gap: px(56), ..default() };
    let content = commands.spawn((ChildOf(panel), content)).id();

    let left = commands.spawn((ChildOf(content), Node { flex_direction: FlexDirection::Column, row_gap: px(22), width: px(440), flex_shrink: 0.0, ..default() })).id();
    commands.spawn((
        ChildOf(left),
        Node { flex_direction: FlexDirection::Column, ..default() },
        children![
            (Title, Text::new("MULTISLOP"), display_font(88.0), LineHeight::RelativeToFont(0.95), TextColor(INK), TextShadow { offset: Vec2::new(6.0, 6.0), color: you }),
            (Text::new("LAN GAMES, MADE OF SLOP"), ui_font(13.0, FontWeight::BOLD), LetterSpacing::Px(6.0), TextColor(DIM)),
        ],
    ));
    let card = Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(14),
        padding: UiRect::all(px(20)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(18)),
        ..default()
    };
    let profile = commands.spawn((ChildOf(left), card.clone(), BackgroundColor(CARD), BorderColor::all(LINE))).id();
    commands.spawn((ChildOf(profile), label("PLAYING AS")));
    let dot = Node { width: px(22), height: px(22), border_radius: BorderRadius::MAX, ..default() };
    commands.spawn((
        ChildOf(profile),
        Node { column_gap: px(14), align_items: AlignItems::Center, ..default() },
        children![
            (NameDot, dot, BackgroundColor(you), BoxShadow::new(you, px(0), px(0), px(2), px(14))),
            (NameText, Text::new(me.0.name.clone()), ui_font(32.0, FontWeight::BOLD), TextColor(you)),
        ],
    ));
    commands.spawn((ChildOf(profile), Swatches, Node { width: px(351), flex_wrap: FlexWrap::Wrap, column_gap: px(9), row_gap: px(9), ..default() }));
    commands.spawn((ChildOf(profile), Text::new("Your color, in every game."), ui_font(13.0, FontWeight::MEDIUM), TextColor(FAINT)));

    let button = Node {
        padding: UiRect::axes(px(24), px(16)),
        justify_content: JustifyContent::Center,
        border_radius: BorderRadius::all(px(16)),
        ..default()
    };
    let fill = LinearGradient::to_right(vec![ColorStop::auto(HOT_PINK), ColorStop::auto(HOT_ORANGE)]);
    let host = commands.spawn((
        ChildOf(left),
        button,
        BackgroundGradient::from(fill),
        BoxShadow::new(HOT_PINK.with_alpha(0.45), px(0), px(8), px(0), px(28)),
        children![(Text::new("HOST A LAN GAME"), ui_font(19.0, FontWeight::BLACK), LetterSpacing::Px(3.0), TextColor(Color::WHITE), Pickable::IGNORE)],
    ));
    hoverable(host, Color::NONE, Color::NONE, true).observe(|_: On<PointerClick>, mut b: ResMut<Browser>| b.host_clicked = true);
    commands.spawn((ChildOf(left), HostList, Node { flex_direction: FlexDirection::Column, row_gap: px(6), ..default() }));

    let right = commands.spawn((ChildOf(content), Node { flex_direction: FlexDirection::Column, row_gap: px(18), flex_grow: 1.0, max_width: px(860), ..default() })).id();
    let dot = Node { width: px(9), height: px(9), border_radius: BorderRadius::MAX, ..default() };
    commands.spawn((
        ChildOf(right),
        Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::FlexEnd, margin: UiRect::top(px(28)), ..default() },
        children![
            (Text::new("SERVERS"), display_font(54.0), TextColor(INK)),
            (
                Node { column_gap: px(10), align_items: AlignItems::Center, margin: UiRect::bottom(px(12)), ..default() },
                children![
                    (StatusDot, dot, BackgroundColor(Color::srgb(0.2, 1.0, 0.5)), BoxShadow::new(Color::srgb(0.2, 1.0, 0.5), px(0), px(0), px(1), px(8))),
                    (Text::new("scanning your network"), ui_font(14.0, FontWeight::MEDIUM), TextColor(DIM)),
                ],
            ),
        ],
    ));
    commands.spawn((
        ChildOf(right),
        List,
        Node { min_height: px(330), padding: UiRect::all(px(10)), row_gap: px(4), ..card },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.25)),
        BorderColor::all(LINE),
        BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.5), px(0), px(20), px(0), px(50)),
    ));
    let field = Node {
        padding: UiRect::axes(px(16), px(13)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(12)),
        flex_grow: 1.0,
        ..default()
    };
    commands.spawn((
        ChildOf(right),
        Node { flex_direction: FlexDirection::Column, row_gap: px(8), ..default() },
        children![
            label("DIRECT CONNECT"),
            (field, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)), BorderColor::all(LINE), children![(AddressText, Text::new(""), ui_font(20.0, FontWeight::MEDIUM), TextColor(INK))]),
        ],
    ));
    commands.spawn((ChildOf(right), ErrorText, Text::new(""), ui_font(15.0, FontWeight::SEMIBOLD), TextColor(BAD)));
    let hint = "Click a server, or pick one with the arrow keys and press Enter. Or type an address and press Enter.";
    commands.spawn((ChildOf(right), Text::new(hint), ui_font(13.0, FontWeight::MEDIUM), TextColor(FAINT)));
}

/// Drifts the glows, pulses the scanning light, and restyles everything in the player's color when they pick another.
pub fn animate(
    mut commands: Commands,
    me: Res<Me>,
    mut b: ResMut<Browser>,
    mut orbs: Query<(&Orb, &mut UiTransform, &mut BackgroundGradient)>,
    mut status: Single<&mut BackgroundColor, With<StatusDot>>,
    mut title: Single<&mut TextShadow, With<Title>>,
    mut name: Single<&mut TextColor, With<NameText>>,
    mut dot: Single<(&mut BackgroundColor, &mut BoxShadow), (With<NameDot>, Without<StatusDot>)>,
    swatches: Single<Entity, With<Swatches>>,
) {
    let t = clock() as f32;
    for (orb, mut transform, _) in &mut orbs {
        let phase = orb.0 as f32 * 1.9;
        transform.translation = Val2::px((t * 0.13 + phase).sin() * 90.0, (t * 0.11 + phase * 1.3).cos() * 70.0);
    }
    status.0 = Color::srgb(0.2, 1.0, 0.5).with_alpha(0.45 + 0.55 * (t * 3.0).sin().abs());
    if !me.is_changed() {
        return;
    }
    let you = accent(&me);
    title.color = you;
    name.0 = you;
    dot.0.0 = you;
    *dot.1 = BoxShadow::new(you, px(0), px(0), px(2), px(14));
    for (orb, _, mut gradient) in &mut orbs {
        if orb.0 == 0 {
            *gradient = glow(you, 0.22);
        }
    }
    b.dirty = true;
    commands.entity(*swatches).despawn_children();
    for color in sdk::PALETTE {
        let [r, g, bl] = color;
        let c = Color::srgb_u8(r, g, bl);
        let picked = color == me.0.color;
        let node = Node { width: px(36), height: px(36), border_radius: BorderRadius::all(px(10)), ..default() };
        let mut swatch = commands.spawn((ChildOf(*swatches), node));
        if picked {
            swatch.insert((Outline { width: px(3), offset: px(3), color: Color::WHITE }, BoxShadow::new(c, px(0), px(0), px(2), px(16))));
        }
        hoverable(swatch, c, c, true).observe(move |_: On<PointerClick>, mut me: ResMut<Me>| {
            me.0.color = color;
            Me::save(color);
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub fn browse(
    mut commands: Commands,
    me: Res<Me>,
    mut net: ResMut<Net>,
    mut b: ResMut<Browser>,
    mut typed: MessageReader<KeyboardInput>,
    mut panel: Single<&mut Visibility, With<Panel>>,
    list: Single<Entity, With<List>>,
    mut address: Single<(&mut Text, &mut TextColor), (With<AddressText>, Without<ErrorText>)>,
    mut error: Single<&mut Text, With<ErrorText>>,
    host_list: Single<Entity, With<HostList>>,
) {
    let browsing = net.server.is_none();
    if !browsing {
        **panel = Visibility::Hidden;
        typed.clear();
        return;
    }
    **panel = Visibility::Inherited;
    let you = accent(&me);
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
        let tile = Node { padding: UiRect::axes(px(16), px(11)), align_items: AlignItems::Center, column_gap: px(12), border_radius: BorderRadius::all(px(12)), ..default() };
        let text = |s: String, size: f32, color: Color| (Text::new(s), ui_font(size, FontWeight::SEMIBOLD), TextColor(color), Pickable::IGNORE);
        let options = commands.spawn((ChildOf(*host_list), Node { column_gap: px(8), ..default() })).id();
        let label = b.players.map_or("Players: auto".to_string(), |n| format!("Players: {n}"));
        let bots = commands.spawn((ChildOf(options), tile.clone(), BackgroundColor(CARD), children![(Node { width: px(120), ..default() }, text(label, 15.0, INK))])).id();
        for (sign, up) in [("−", false), ("+", true)] {
            let pill = Node { width: px(28), height: px(28), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::MAX, ..default() };
            hoverable(commands.spawn((ChildOf(bots), pill, children![text(sign.into(), 17.0, INK)])), CARD_HOT, you.with_alpha(0.5), true)
                .observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| (b.players, b.shown_games) = (step_players(b.players, up), None));
        }
        let (label, base) = if b.rotate { ("Rotate: on", you.with_alpha(0.35)) } else { ("Rotate: off", CARD) };
        hoverable(commands.spawn((ChildOf(options), tile.clone(), children![text(label.into(), 15.0, INK)])), base, CARD_HOT, false)
            .observe(|_: On<PointerClick>, mut b: ResMut<Browser>| (b.rotate, b.shown_games) = (!b.rotate, None));
        if b.rotate {
            commands.spawn((ChildOf(*host_list), text("Each game in turn: 10 rounds, 3 races or 3 matches.".into(), 13.0, FAINT)));
        }
        match &games {
            None => _ = commands.spawn((ChildOf(*host_list), tile, BackgroundColor(CARD), children![text("Finding games...".into(), 16.0, DIM)])),
            Some(list) if list.is_empty() => _ = commands.spawn((ChildOf(*host_list), tile, BackgroundColor(CARD), children![text("No games found to host".into(), 16.0, DIM)])),
            Some(list) => {
                // One tile per game, a chip per map.
                let mut chips = None;
                for (i, game) in list.iter().enumerate() {
                    if i == 0 || list[i - 1].module != game.module {
                        let name = game.label.split_once(" on ").map_or(&*game.label, |(n, _)| n);
                        let row = commands.spawn((ChildOf(*host_list), Node { flex_wrap: FlexWrap::Wrap, ..tile.clone() }, BackgroundColor(CARD))).id();
                        commands.spawn((ChildOf(row), Node { width: px(150), ..default() }, text(name.to_uppercase(), 17.0, INK)));
                        chips = Some(row);
                    }
                    let chip = Node { padding: UiRect::axes(px(12), px(5)), border_radius: BorderRadius::MAX, ..default() };
                    let chip = commands.spawn((ChildOf(chips.unwrap()), chip, children![text(game.map.clone(), 14.0, INK)]));
                    hoverable(chip, CARD_HOT, you.with_alpha(0.6), true).observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| b.host_pick = Some(i));
                }
            }
        }
    }
    if let Some(game) = b.host_pick.take().and_then(|i| games.as_ref()?.get(i).cloned()) {
        crate::client::store(&game.module);
        let mut modules = vec![game.module];
        // Rotating: then each other game once, in list order.
        for other in games.iter().flatten().filter(|_| b.rotate) {
            if !modules.contains(&other.module) {
                modules.push(other.module.clone());
            }
        }
        match crate::server::host(format!("{}'s game", me.0.name), modules, game.map, b.players) {
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
    let caret = if now % 1.0 < 0.5 { "|" } else { "" };
    let (text, color) = &mut *address;
    (text.0, color.0) = if b.address.is_empty() { (format!("{caret}host or host:port"), FAINT) } else { (format!("{}{caret}", b.address), INK) };
    error.0 = b.error.clone();
    if !b.dirty {
        return;
    }
    b.dirty = false;
    commands.entity(*list).despawn_children();
    let header = ["NAME", "GAME", "PLAYERS", "PING"].map(|h| (h.to_string(), FAINT));
    row(&mut commands, *list, header, 12.0, None);
    if b.found.is_empty() {
        commands.spawn((
            ChildOf(*list),
            Node { flex_direction: FlexDirection::Column, row_gap: px(6), padding: UiRect::all(px(16)), ..default() },
            children![
                (Text::new("Nobody's playing yet."), ui_font(20.0, FontWeight::SEMIBOLD), TextColor(DIM)),
                (Text::new("Host a game and it shows up here for everyone on your network."), ui_font(14.0, FontWeight::MEDIUM), TextColor(FAINT)),
            ],
        ));
    }
    let selected = b.selected.min(b.found.len().saturating_sub(1));
    for (i, (addr, f)) in b.sorted().into_iter().enumerate() {
        let cells = match &f.game {
            Some(g) => {
                let ping = match f.ping_ms {
                    p if p < 40.0 => Color::srgb(0.3, 1.0, 0.55),
                    p if p < 100.0 => Color::srgb(1.0, 0.85, 0.3),
                    _ => BAD,
                };
                [
                    (f.name.clone(), INK),
                    (format!("{} · {}", g.game, g.map), DIM),
                    (format!("{} + {} bots", g.players, g.bots), DIM),
                    (format!("{:.0} ms", f.ping_ms), ping),
                ]
            }
            None => [(f.name.clone(), FAINT), (String::new(), FAINT), (format!("build {}", f.build.as_deref().unwrap_or("?")), FAINT), (String::new(), FAINT)],
        };
        let r = row(&mut commands, *list, cells, 17.0, (i == selected).then_some(you));
        let base = if i == selected { you.with_alpha(0.16) } else { Color::srgba(1.0, 1.0, 1.0, 0.025) };
        let mut r = hoverable(commands.entity(r), base, you.with_alpha(0.26), false);
        if f.game.is_some() {
            r.observe(move |_: On<PointerClick>, mut b: ResMut<Browser>| b.clicked = Some(addr));
        }
    }
}

/// The Esc menu in a game.
#[derive(Resource, Default)]
pub struct Paused {
    pub open: bool,
    /// A button clicked, acted on next frame (Disconnect by the watchdog, which tears the game down).
    pub pick: Option<Pick>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Pick {
    Resume,
    Disconnect,
    Quit,
}

#[derive(Component)]
pub struct PauseMenu;

pub fn setup_pause(mut commands: Commands) {
    let full = Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        row_gap: px(14),
        ..default()
    };
    let shade = RadialGradient::new(UiPosition::CENTER, RadialGradientShape::FarthestCorner, vec![
        ColorStop::auto(Color::srgba(0.05, 0.02, 0.1, 0.7)),
        ColorStop::auto(Color::srgba(0.0, 0.0, 0.0, 0.92)),
    ]);
    let menu = commands.spawn((PauseMenu, Visibility::Hidden, GlobalZIndex(20), full, BackgroundGradient::from(shade))).id();
    commands.spawn((ChildOf(menu), Text::new("PAUSED"), display_font(80.0), TextColor(INK), Node { margin: UiRect::bottom(px(18)), ..default() }));
    let button = Node {
        width: px(320),
        padding: UiRect::vertical(px(15)),
        justify_content: JustifyContent::Center,
        border_radius: BorderRadius::all(px(14)),
        ..default()
    };
    for (text, pick) in [("RESUME", Pick::Resume), ("DISCONNECT", Pick::Disconnect), ("QUIT GAME", Pick::Quit)] {
        let label = (Text::new(text), ui_font(18.0, FontWeight::BLACK), LetterSpacing::Px(3.0), TextColor(Color::WHITE), Pickable::IGNORE);
        let b = commands.spawn((ChildOf(menu), button.clone(), children![label]));
        let b = match pick {
            Pick::Resume => {
                let mut b = hoverable(b, Color::NONE, Color::NONE, true);
                b.insert((BackgroundGradient::from(LinearGradient::to_right(vec![ColorStop::auto(HOT_PINK), ColorStop::auto(HOT_ORANGE)])), BoxShadow::new(HOT_PINK.with_alpha(0.45), px(0), px(8), px(0), px(28))));
                b
            }
            _ => hoverable(b, CARD_HOT, Color::srgba(1.0, 1.0, 1.0, 0.2), true),
        };
        let mut b = b;
        b.observe(move |_: On<PointerClick>, mut p: ResMut<Paused>| p.pick = Some(pick));
    }
    commands.spawn((ChildOf(menu), Text::new("Esc to go back"), ui_font(13.0, FontWeight::MEDIUM), TextColor(FAINT), Node { margin: UiRect::top(px(10)), ..default() }));
}

/// Esc in a game opens and closes the menu; Resume grabs the mouse again, Quit closes the game.
pub fn pause(
    mut paused: ResMut<Paused>,
    net: Res<Net>,
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: Single<&mut Visibility, With<PauseMenu>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if net.server.is_none() {
        *paused = Paused::default();
    } else if keys.just_pressed(KeyCode::Escape) {
        paused.open = !paused.open;
        if !paused.open {
            paused.pick = Some(Pick::Resume);
        }
    }
    match paused.pick {
        Some(Pick::Resume) => {
            *paused = Paused::default();
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            cursor.visible = false;
        }
        Some(Pick::Quit) => _ = exit.write(AppExit::Success),
        _ => {}
    }
    **menu = if paused.open { Visibility::Inherited } else { Visibility::Hidden };
}

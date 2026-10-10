//! Instaslop, instagib deathmatch: everyone for themselves with a one-shot railgun, first to the frag limit or most
//! frags when time runs out. Bots fill the arena up to `FILL` players while anyone is playing.

use crate::bot::{Bot, Seen};
use crate::map::{MAP, MAPS};
use crate::shared::*;
use glam::Vec3;
use sdk::{Entity, Frame, GameInfo, History, ServerGame, log};
use std::collections::BTreeMap;

pub const FRAG_LIMIT: i32 = 25;
const MATCH_SECS: f32 = 600.0;
const INTERMISSION_SECS: f32 = 8.0;
/// Matches per turn when the host rotates games.
const ROTATE_AFTER: u32 = 3;
/// Players in the arena while any human is, unless the host sets a bot count: bots make up the rest.
pub const FILL: usize = 8;
/// Dead for at least this long; fire respawns after it, and you come back on your own after `RESPAWN_AUTO`.
const RESPAWN_MIN: f32 = 1.0;
const RESPAWN_AUTO: f32 = 3.0;

type Players = BTreeMap<u32, Player>;

struct Player {
    id: u32,
    state: PlayerState,
    alive: bool,
    died_at: u32,
    frags: i32,
    deaths: u32,
    /// Inputs received since the last tick, applied in order on the next one.
    inputs: Vec<Input>,
    /// Bumped on every respawn; see `sdk::Entity::teleport`.
    teleport: u8,
    bot: Option<Bot>,
    name: String,
    color: [u8; 3],
    ping: u16,
}

impl Player {
    fn new(id: u32, who: sdk::Profile, bot: Option<Bot>) -> Player {
        let (name, color) = (who.name, who.color);
        Player { id, state: PlayerState::default(), alive: false, died_at: 0, frags: 0, deaths: 0, inputs: vec![], teleport: 0, bot, name, color, ping: 0 }
    }
}

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

pub struct Arena {
    map: String,
    players: Players,
    /// Positions of every player per tick, for lag compensation.
    history: History,
    tick: u32,
    next_id: u32,
    /// The match (or intermission) ends at this tick.
    ends: u32,
    winner: Option<u32>,
    events: Vec<Event>,
    /// Bots to play with, set by the host; None fills up to `FILL` players.
    bots: Option<usize>,
    /// Matches played to the end of their intermission since `set_map`, for rotating games.
    played: u32,
}

impl Default for Arena {
    fn default() -> Arena {
        Arena {
            map: MAPS[0].into(),
            players: Players::new(),
            history: History::new(MAX_REWIND_TICKS as usize + 8),
            tick: 0,
            next_id: 1,
            ends: ticks(MATCH_SECS),
            winner: None,
            events: vec![],
            bots: None,
            played: 0,
        }
    }
}

impl Arena {
    pub fn new() -> Arena {
        Arena::default()
    }
}

impl ServerGame for Arena {
    fn connect(&mut self, who: &sdk::Profile) -> u32 {
        let (id, name) = (self.next_id, clean_name(&who.name));
        self.next_id += 1;
        log(&format!("{name} (player {id}) joined"));
        let mut p = Player::new(id, sdk::Profile { name, ..who.clone() }, None);
        spawn(&mut p, &self.players, self.tick);
        self.players.insert(id, p);
        id
    }

    fn disconnect(&mut self, id: u32) {
        if let Some(p) = self.players.remove(&id) {
            log(&format!("{} left", p.name));
        }
    }

    fn ping(&mut self, id: u32, ms: u16) {
        if let Some(p) = self.players.get_mut(&id) {
            p.ping = ms;
        }
    }

    fn input(&mut self, id: u32, seq: u32, view_tick: f32, input: &[u8]) {
        if let (Some(p), Some(inp)) = (self.players.get_mut(&id), decode::<Input>(input)) {
            p.inputs.push(Input { seq, view_tick, ..inp });
        }
    }

    fn tick(&mut self) -> Frame {
        fill_bots(&mut self.players, &mut self.next_id, self.tick, self.bots);
        run_inputs(&mut self.players, &self.history, self.tick, &mut self.events);
        self.tick += 1;
        self.update_match();
        let entities: Vec<Entity> = self.players.values().map(net_player).map(|(p, t)| p.entity(t)).collect();
        self.history.record(self.tick, &entities);
        let state = Match { time_left: self.ends.saturating_sub(self.tick) as f32 / TICK_HZ as f32, frag_limit: FRAG_LIMIT, winner: self.winner };
        let events = self.events.drain(..).map(|e| sdk::Event { to: None, data: encode(&e) }).collect();
        Frame { map: self.map.clone(), state: encode(&state), entities, events, finished: self.played >= ROTATE_AFTER }
    }

    fn info(&mut self) -> GameInfo {
        let bots = self.players.values().filter(|p| p.bot.is_some()).count() as u32;
        let maps = MAPS.map(String::from).to_vec();
        GameInfo { game: "Instaslop".into(), map: self.map.clone(), maps, players: self.players.len() as u32 - bots, bots }
    }

    fn set_map(&mut self, map: &str) {
        if !MAPS.contains(&map) {
            return log(&format!("no map called {map}; playing {}", self.map));
        }
        (self.map, self.played) = (map.into(), 0);
        self.restart();
    }

    fn set_bots(&mut self, bots: u32) {
        log(&format!("{bots} bots"));
        self.bots = Some(bots as usize);
    }
}

impl Arena {
    /// Ends the match on the frag limit or the clock, and starts the next one after the intermission.
    fn update_match(&mut self) {
        if self.players.is_empty() {
            self.ends = self.tick + ticks(MATCH_SECS);
            return;
        }
        let leader = self.players.values().max_by_key(|p| (p.frags, std::cmp::Reverse(p.deaths))).map(|p| (p.id, p.frags));
        match self.winner {
            None if self.tick >= self.ends || leader.is_some_and(|(_, f)| f >= FRAG_LIMIT) => {
                self.winner = leader.map(|(id, _)| id);
                self.ends = self.tick + ticks(INTERMISSION_SECS);
                for p in self.players.values_mut() {
                    p.state.frozen = true;
                }
                if let Some(w) = self.winner.and_then(|id| self.players.get(&id)) {
                    log(&format!("{} wins with {} frags", w.name, w.frags));
                }
            }
            Some(_) if self.tick >= self.ends => {
                self.played += 1;
                self.restart();
            }
            _ => {}
        }
    }

    fn restart(&mut self) {
        (self.winner, self.ends) = (None, self.tick + ticks(MATCH_SECS));
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for id in ids {
            let mut p = self.players.remove(&id).unwrap();
            (p.frags, p.deaths, p.state.frozen) = (0, 0, false);
            spawn(&mut p, &self.players, self.tick);
            self.players.insert(id, p);
        }
    }
}

fn net_player(p: &Player) -> (NetPlayer, u8) {
    let n = NetPlayer { id: p.id, state: p.state, alive: p.alive, frags: p.frags, deaths: p.deaths, bot: p.bot.is_some(), name: p.name.clone(), color: p.color, ping: p.ping };
    (n, p.teleport)
}

/// Brings a player back at the spawn point furthest from everyone alive (one of the best few, so it isn't predictable).
fn spawn(p: &mut Player, others: &Players, tick: u32) {
    let threat = |s: &Vec3| others.values().filter(|o| o.alive && o.id != p.id).map(|o| o.state.pos.distance(*s)).fold(f32::MAX, f32::min);
    let mut spots: Vec<Vec3> = MAP.spawns.clone();
    spots.sort_by(|a, b| threat(b).total_cmp(&threat(a)));
    let pos = spots[(tick.wrapping_mul(2654435761) >> 16) as usize % 3];
    // Face the middle of the arena.
    let yaw = f32::atan2(pos.x, pos.z);
    p.state = PlayerState { pos, yaw, frozen: p.state.frozen, ..Default::default() };
    p.alive = true;
    p.teleport = p.teleport.wrapping_add(1);
}

/// One tick of play: bots decide, then every player's inputs since the last tick are applied.
fn run_inputs(players: &mut Players, history: &History, tick: u32, events: &mut Vec<Event>) {
    let seen: Vec<Seen> = players.values().map(|p| Seen { id: p.id, pos: p.state.pos, alive: p.alive }).collect();
    for p in players.values_mut() {
        if let Some(bot) = &mut p.bot {
            p.inputs.push(bot.think(p.id, &p.state, p.alive, &seen, tick));
        }
    }
    let ids: Vec<u32> = players.keys().copied().collect();
    for id in ids {
        let inputs = std::mem::take(&mut players.get_mut(&id).unwrap().inputs);
        let dead_for = |p: &Player| tick.saturating_sub(p.died_at);
        for inp in inputs {
            let p = players.get_mut(&id).unwrap();
            if !p.alive {
                if inp.fire && dead_for(p) >= ticks(RESPAWN_MIN) && !p.state.frozen {
                    respawn(players, id, tick);
                }
                continue;
            }
            let Some(dir) = step_player(&mut p.state, &inp).fired else { continue };
            let from = eye(&p.state);
            let to = rail(players, history, tick, id, (from, dir), inp.view_tick, events);
            events.push(Event::Rail { shooter: id, from, to });
        }
        let p = &players[&id];
        if !p.alive && dead_for(p) >= ticks(RESPAWN_AUTO) && !p.state.frozen {
            respawn(players, id, tick);
        }
    }
}

fn respawn(players: &mut Players, id: u32, tick: u32) {
    let mut p = players.remove(&id).unwrap();
    spawn(&mut p, players, tick);
    players.insert(id, p);
}

/// Lag-compensated rail: rewinds everyone else to where the shooter saw them and frags all of them along the beam,
/// up to the first wall. Returns where the beam ended.
fn rail(players: &mut Players, history: &History, tick: u32, shooter: u32, (o, d): (Vec3, Vec3), view_tick: f32, events: &mut Vec<Event>) -> Vec3 {
    let view_tick = view_tick.clamp(tick as f32 - MAX_REWIND_TICKS, tick as f32);
    let wall = trace_world(o, d).0;
    let hit: Vec<u32> = players
        .values()
        .filter(|p| p.id != shooter && p.alive)
        .filter(|p| {
            let now = Entity { id: p.id, pos: p.state.pos, teleport: p.teleport, ..Default::default() };
            history.rewind(&now, view_tick).and_then(|then| trace_capsule(o, d, then.pos)).is_some_and(|t| t < wall)
        })
        .map(|p| p.id)
        .collect();
    for &victim in &hit {
        let v = players.get_mut(&victim).unwrap();
        (v.alive, v.died_at, v.deaths) = (false, tick, v.deaths + 1);
        players.get_mut(&shooter).unwrap().frags += 1;
        events.push(Event::Frag { killer: shooter, victim });
    }
    o + d * wall
}

/// Printable, at most 16 characters, never empty.
fn clean_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(16).collect();
    if name.trim().is_empty() { "player".into() } else { name.trim().into() }
}

fn bot_name(id: u32) -> String {
    const NAMES: [&str; 12] = ["Anarki", "Bitterman", "Crash", "Daemia", "Hunter", "Klesk", "Major", "Orbb", "Ranger", "Sarge", "Slash", "Xaero"];
    format!("BOT {}", NAMES[id as usize % NAMES.len()])
}

/// Adds or removes bots so there are `bots` of them (default: enough to make `FILL` players) while any human is
/// playing, and none without.
fn fill_bots(players: &mut Players, next_id: &mut u32, tick: u32, bots: Option<usize>) {
    let humans = players.values().filter(|p| p.bot.is_none()).count();
    let want = if humans == 0 { 0 } else { bots.unwrap_or(FILL.saturating_sub(humans)) };
    let mut bots: Vec<u32> = players.values().filter(|p| p.bot.is_some()).map(|p| p.id).collect();
    while bots.len() > want {
        players.remove(&bots.pop().unwrap());
    }
    for _ in bots.len()..want {
        let id = *next_id;
        *next_id += 1;
        let mut p = Player::new(id, bot_name(id).as_str().into(), Some(Bot::new(id)));
        p.state.frozen = players.values().any(|p| p.state.frozen);
        spawn(&mut p, players, tick + id);
        log(&format!("{} joined", p.name));
        players.insert(id, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_frags_everyone_along_the_beam_until_a_wall() {
        let mut game = Arena::new();
        let (a, b, c) = (game.connect(&"a".into()), game.connect(&"b".into()), game.connect(&"c".into()));
        let start = MAP.center(20, 8);
        for (id, x) in [(a, 0.0), (b, 6.0), (c, 12.0)] {
            game.players.get_mut(&id).unwrap().state.pos = start + Vec3::X * x;
        }
        let mut events = vec![];
        let to = rail(&mut game.players, &History::new(0), 10, a, (start + Vec3::Y * EYE, Vec3::X), 10.0, &mut events);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(!game.players[&b].alive && !game.players[&c].alive && game.players[&a].frags == 2);
        assert!(to.x > start.x + 12.0, "the beam goes on to the wall");
    }

    #[test]
    fn dead_respawn_on_fire_after_a_second_or_on_their_own() {
        let mut game = Arena::new();
        let id = game.connect(&"a".into());
        game.players.get_mut(&id).unwrap().alive = false;
        game.players.get_mut(&id).unwrap().died_at = game.tick;
        let fire = Input { fire: true, ..Default::default() };
        game.input(id, 1, 0.0, &encode(&fire));
        game.tick();
        assert!(!game.players[&id].alive, "too soon");
        for _ in 0..ticks(RESPAWN_MIN) {
            game.tick();
        }
        game.input(id, 2, 0.0, &encode(&fire));
        game.tick();
        assert!(game.players[&id].alive);
    }

    /// A full arena of bots for a few minutes, no network: they should roam, find each other and frag.
    #[test]
    fn bots_play_a_match() {
        let mut game = Arena::new();
        game.connect(&"observer".into());
        game.players.values_mut().for_each(|p| p.alive = false);
        game.players.values_mut().for_each(|p| p.died_at = u32::MAX / 2);
        let (mut rails, mut frags) = (0, 0);
        for _ in 0..ticks(180.0) {
            for e in game.tick().events {
                match decode::<Event>(&e.data) {
                    Some(Event::Rail { .. }) => rails += 1,
                    Some(Event::Frag { .. }) => frags += 1,
                    None => {}
                }
            }
            for p in game.players.values() {
                assert!(!p.alive || in_bounds(p.state.pos), "{} at {:?}", p.name, p.state.pos);
            }
        }
        let bots = game.players.values().filter(|p| p.bot.is_some()).count();
        println!("{bots} bots, 3 min: {rails} rails, {frags} frags ({:.0}% hit)", 100.0 * frags as f32 / rails.max(1) as f32);
        assert_eq!(bots, FILL - 1);
        assert!(frags >= 20, "bots should find and frag each other: {frags}");
    }

    #[test]
    fn bot_count_is_settable() {
        let mut game = Arena::new();
        game.set_bots(12);
        game.connect(&"a".into());
        game.tick();
        assert_eq!(game.players.values().filter(|p| p.bot.is_some()).count(), 12);
        game.set_bots(0);
        game.tick();
        assert_eq!(game.players.len(), 1);
    }

    #[test]
    fn frag_limit_ends_the_match_and_the_next_starts_fresh() {
        let mut game = Arena::new();
        let id = game.connect(&"a".into());
        game.players.get_mut(&id).unwrap().frags = FRAG_LIMIT;
        game.tick();
        assert_eq!(game.winner, Some(id));
        assert!(game.players.values().all(|p| p.state.frozen));
        for _ in 0..ticks(INTERMISSION_SECS) + 1 {
            game.tick();
        }
        assert_eq!(game.winner, None);
        assert!(game.players.values().all(|p| p.frags == 0 && !p.state.frozen));
    }
}

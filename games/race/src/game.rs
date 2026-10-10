//! Raceslop, an obstacle course race: everyone starts on the line together and runs for the finish. Lava and laser
//! bars send you back to your last checkpoint. The race ends when every person is in, `CLOSING_SECS` after the first
//! finisher, or at `RACE_SECS`; then the results, and the next race, on a new course when the map is "random". Bots
//! fill up to `FILL` runners.

use crate::bot::Bot;
use crate::map::{self, MAPS, course};
use crate::shared::*;
use glam::Vec3;
use sdk::{Entity, Frame, GameInfo, ServerGame, log};
use std::collections::BTreeMap;

const COUNTDOWN_SECS: f32 = 4.0;
const RACE_SECS: f32 = 300.0;
const CLOSING_SECS: f32 = 30.0;
const RESULTS_SECS: f32 = 8.0;
/// Races per turn when the host rotates games.
const ROTATE_AFTER: u32 = 3;
/// Runners while any human is on, unless the host sets a bot count: bots make up the rest.
pub const FILL: usize = 6;

type Players = BTreeMap<u32, Player>;

struct Player {
    id: u32,
    state: PlayerState,
    checkpoint: u8,
    /// Where to come back after falling: the spot of the last checkpoint reached, or the start line.
    respawn: Vec3,
    finish: Option<(u8, f32)>,
    wins: u32,
    /// Inputs received since the last tick, applied in order on the next one.
    inputs: Vec<Input>,
    /// Bumped on every teleport; see `sdk::Entity::teleport`.
    teleport: u8,
    bot: Option<Bot>,
    name: String,
    color: [u8; 3],
    ping: u16,
}

impl Player {
    fn new(id: u32, who: sdk::Profile, bot: Option<Bot>) -> Player {
        let (state, name, color) = (PlayerState::default(), who.name, who.color);
        Player { id, state, checkpoint: 0, respawn: Vec3::ZERO, finish: None, wins: 0, inputs: vec![], teleport: 0, bot, name, color, ping: 0 }
    }

    fn net(&self) -> NetPlayer {
        let (id, state, checkpoint, finish, wins, ping) = (self.id, self.state, self.checkpoint, self.finish, self.wins, self.ping);
        NetPlayer { id, state, checkpoint, finish, wins, bot: self.bot.is_some(), name: self.name.clone(), color: self.color, ping }
    }

    /// To `pos` facing north, standing still.
    fn teleport(&mut self, pos: Vec3) {
        self.state = PlayerState { pos, on_ground: true, frozen: self.state.frozen, ..Default::default() };
        self.teleport = self.teleport.wrapping_add(1);
    }
}

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

pub struct Gauntlet {
    /// One of `MAPS`, as the host picked it.
    choice: String,
    /// The course being raced: `choice`, or for "random" the seed this race's course was dealt from.
    map: String,
    /// Mixed into the next random course's seed: ping times, so courses differ from one server run to the next.
    entropy: u32,
    players: Players,
    tick: u32,
    next_id: u32,
    phase: Phase,
    /// The tick this race's countdown began, and when the current phase ends.
    began: u32,
    ends: u32,
    events: Vec<Event>,
    /// Bots to race, set by the host; None fills up to `FILL` runners.
    bots: Option<usize>,
    /// Races run to the end of their results since `set_map`, for rotating games.
    played: u32,
}

impl Default for Gauntlet {
    fn default() -> Gauntlet {
        let mut game = Gauntlet {
            choice: MAPS[0].into(),
            map: String::new(),
            entropy: 0,
            players: Players::new(),
            tick: 0,
            next_id: 1,
            phase: Phase::Countdown,
            began: 0,
            ends: ticks(COUNTDOWN_SECS),
            events: vec![],
            bots: None,
            played: 0,
        };
        game.new_course();
        game
    }
}

impl Gauntlet {
    pub fn new() -> Gauntlet {
        Gauntlet::default()
    }
}

impl ServerGame for Gauntlet {
    fn connect(&mut self, who: &sdk::Profile) -> u32 {
        let (id, name) = (self.next_id, clean_name(&who.name));
        self.next_id += 1;
        // The first one in gets a fresh course rather than whatever the empty server last dealt.
        if self.players.values().all(|p| p.bot.is_some()) {
            self.new_course();
        }
        log(&format!("{name} (player {id}) joined"));
        let mut p = Player::new(id, sdk::Profile { name, ..who.clone() }, None);
        self.line_up(&mut p);
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
        self.entropy = self.entropy.rotate_left(5) ^ ms as u32;
    }

    fn input(&mut self, id: u32, _seq: u32, _view_tick: f32, input: &[u8]) {
        if let (Some(p), Some(inp)) = (self.players.get_mut(&id), decode::<Input>(input)) {
            p.inputs.push(inp);
        }
    }

    fn tick(&mut self) -> Frame {
        self.fill_bots();
        self.run_inputs();
        self.tick += 1;
        self.update_race();
        let entities: Vec<Entity> = self.players.values().map(|p| p.net().entity(p.teleport)).collect();
        let secs = |t: u32| t as f32 / TICK_HZ as f32;
        let (time, closing) = match self.phase {
            Phase::Racing => (secs(self.tick - self.began - ticks(COUNTDOWN_SECS)), self.first_in().then(|| secs(self.ends.saturating_sub(self.tick)))),
            _ => (secs(self.ends.saturating_sub(self.tick)), None),
        };
        let state = Race { phase: self.phase, clock: self.tick - self.began, time, closing };
        let events = self.events.drain(..).map(|e| sdk::Event { to: None, data: encode(&e) }).collect();
        Frame { map: self.map.clone(), state: encode(&state), entities, events, finished: self.played >= ROTATE_AFTER }
    }

    fn info(&mut self) -> GameInfo {
        let bots = self.players.values().filter(|p| p.bot.is_some()).count() as u32;
        let maps = MAPS.map(String::from).to_vec();
        GameInfo { game: "Raceslop".into(), map: self.map.clone(), maps, players: self.players.len() as u32 - bots, bots }
    }

    fn set_map(&mut self, map: &str) {
        if !MAPS.contains(&map) && !map.starts_with("random-") {
            return log(&format!("no map called {map}; playing {}", self.map));
        }
        (self.choice, self.played) = (map.into(), 0);
        self.new_course();
        self.restart();
    }

    fn set_bots(&mut self, bots: u32) {
        log(&format!("{bots} bots"));
        self.bots = Some(bots as usize);
    }
}

impl Gauntlet {
    /// Loads the chosen course, dealing a new one for "random".
    fn new_course(&mut self) {
        self.map = match self.choice.as_str() {
            "random" => {
                self.entropy = (self.entropy ^ self.tick).wrapping_mul(0x9E37_79B9).rotate_left(13);
                format!("random-{}", self.entropy % 100_000)
            }
            name => name.into(),
        };
        if !map::load(&self.map) {
            (self.choice, self.map) = ("gauntlet".into(), "gauntlet".into());
            map::load(&self.map);
        }
        log(&format!("course {}", self.map));
    }

    fn first_in(&self) -> bool {
        self.players.values().any(|p| p.finish.is_some())
    }

    /// Onto a free spot on the start line, frozen if the race hasn't started yet.
    fn line_up(&self, p: &mut Player) {
        let taken = |s: &Vec3| self.players.values().any(|o| o.id != p.id && o.state.pos.distance(*s) < 1.0);
        let spot = course().spawns.iter().find(|s| !taken(s)).copied().unwrap_or(course().spawns[p.id as usize % course().spawns.len()]);
        (p.checkpoint, p.respawn, p.finish) = (0, spot, None);
        p.state.frozen = self.phase != Phase::Racing;
        p.teleport(spot);
    }

    fn restart(&mut self) {
        (self.phase, self.began, self.ends) = (Phase::Countdown, self.tick, self.tick + ticks(COUNTDOWN_SECS));
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for id in ids {
            let mut p = self.players.remove(&id).unwrap();
            self.line_up(&mut p);
            self.players.insert(id, p);
        }
    }

    fn update_race(&mut self) {
        if self.players.is_empty() {
            self.restart();
            return;
        }
        let humans_in = self.players.values().filter(|p| p.bot.is_none()).all(|p| p.finish.is_some());
        match self.phase {
            Phase::Countdown if self.tick >= self.ends => {
                (self.phase, self.ends) = (Phase::Racing, self.tick + ticks(RACE_SECS));
                self.players.values_mut().for_each(|p| p.state.frozen = false);
                self.events.push(Event::Go);
            }
            Phase::Racing if self.tick >= self.ends || humans_in => {
                (self.phase, self.ends) = (Phase::Results, self.tick + ticks(RESULTS_SECS));
                self.players.values_mut().for_each(|p| p.state.frozen = true);
                if let Some(w) = self.players.values_mut().find(|p| p.finish.is_some_and(|(place, _)| place == 1)) {
                    w.wins += 1;
                    log(&format!("{} wins", w.name));
                }
            }
            Phase::Results if self.tick >= self.ends => {
                self.played += 1;
                self.new_course();
                self.restart();
            }
            _ => {}
        }
    }

    /// One tick of play: bots decide, then every player's inputs since the last tick are applied, and the course
    /// has its say: lava and lasers send you back, checkpoints and the finish line count.
    fn run_inputs(&mut self) {
        let time = (self.tick - self.began) as f32 / TICK_HZ as f32;
        let racing = self.phase == Phase::Racing;
        for p in self.players.values_mut() {
            if let Some(bot) = &mut p.bot {
                p.inputs.push(bot.think(&p.state, time));
            }
        }
        let mut finished = self.players.values().filter(|p| p.finish.is_some()).count() as u8;
        for p in self.players.values_mut() {
            for inp in std::mem::take(&mut p.inputs) {
                step_player(&mut p.state, &inp);
                let (pos, lava) = (p.state.pos, p.state.pos.y < LAVA);
                let laser = lasered(pos, time);
                if lava || laser || (inp.reset && !p.state.frozen) {
                    if lava || laser {
                        self.events.push(Event::Fell { id: p.id, at: pos, lasered: laser });
                    }
                    p.teleport(p.respawn);
                    continue;
                }
                let (x, z) = course().cell(pos);
                if !p.state.on_ground || !racing {
                    continue;
                }
                if let Some(n) = course().checkpoint(x, z).filter(|&n| n > p.checkpoint) {
                    (p.checkpoint, p.respawn) = (n, course().ground(x, z));
                    self.events.push(Event::Checkpoint { id: p.id, n });
                }
                if course().is_finish(x, z) && p.finish.is_none() {
                    finished += 1;
                    let secs = (self.tick - self.began - ticks(COUNTDOWN_SECS)) as f32 / TICK_HZ as f32;
                    p.finish = Some((finished, secs));
                    self.events.push(Event::Finish { id: p.id, place: finished, secs });
                    log(&format!("{} finishes {finished} in {secs:.2} s", p.name));
                    if finished == 1 {
                        self.ends = self.ends.min(self.tick + ticks(CLOSING_SECS));
                    }
                }
            }
        }
    }

    /// Adds or removes bots so there are `bots` of them (default: enough to make `FILL` runners) while any human is
    /// on, and none without.
    fn fill_bots(&mut self) {
        let humans = self.players.values().filter(|p| p.bot.is_none()).count();
        let want = if humans == 0 { 0 } else { self.bots.unwrap_or(FILL.saturating_sub(humans)) };
        let mut bots: Vec<u32> = self.players.values().filter(|p| p.bot.is_some()).map(|p| p.id).collect();
        while bots.len() > want {
            self.players.remove(&bots.pop().unwrap());
        }
        for _ in bots.len()..want {
            let id = self.next_id;
            self.next_id += 1;
            let mut p = Player::new(id, bot_name(id).as_str().into(), Some(Bot::new(id)));
            self.line_up(&mut p);
            log(&format!("{} joined", p.name));
            self.players.insert(id, p);
        }
    }
}

/// Printable, at most 16 characters, never empty.
fn clean_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(16).collect();
    if name.trim().is_empty() { "player".into() } else { name.trim().into() }
}

fn bot_name(id: u32) -> String {
    const NAMES: [&str; 12] = ["Bolt", "Dash", "Flick", "Hopper", "Jinx", "Nimble", "Pogo", "Quill", "Scamper", "Skip", "Sprout", "Zip"];
    format!("BOT {}", NAMES[id as usize % NAMES.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the game, people standing still.
    fn race(game: &mut Gauntlet, secs: f32) -> Vec<Event> {
        let humans: Vec<u32> = game.players.values().filter(|p| p.bot.is_none()).map(|p| p.id).collect();
        let mut events = vec![];
        for _ in 0..ticks(secs) {
            humans.iter().for_each(|&id| game.input(id, 0, 0.0, &encode(&Input::default())));
            events.extend(game.tick().events.iter().filter_map(|e| decode::<Event>(&e.data)));
        }
        events
    }

    #[test]
    fn frozen_on_the_line_until_go() {
        let mut game = Gauntlet::new();
        let id = game.connect(&"a".into());
        let start = game.players[&id].state.pos;
        game.input(id, 1, 0.0, &encode(&Input { forward: 1.0, ..Default::default() }));
        game.tick();
        assert_eq!(game.players[&id].state.pos, start);
        assert!(race(&mut game, COUNTDOWN_SECS).iter().any(|e| matches!(e, Event::Go)));
        assert!(!game.players[&id].state.frozen);
    }

    #[test]
    fn lava_sends_you_back_to_the_last_checkpoint() {
        let mut game = Gauntlet::new();
        game.set_map("gauntlet");
        game.set_bots(0);
        let id = game.connect(&"a".into());
        race(&mut game, COUNTDOWN_SECS + 0.1);
        let z = (0..course().depth()).rev().find(|&z| course().checkpoint(7, z) == Some(1)).unwrap();
        let cp = course().ground(7, z);
        game.players.get_mut(&id).unwrap().teleport(cp);
        race(&mut game, 0.05);
        assert_eq!(game.players[&id].checkpoint, 1);
        game.players.get_mut(&id).unwrap().teleport(cp.with_y(0.0) + Vec3::Z * 20.0);
        let events = race(&mut game, 0.1);
        assert!(events.iter().any(|e| matches!(e, Event::Fell { lasered: false, .. })), "{events:?}");
        assert_eq!(game.players[&id].state.pos, cp);
    }

    #[test]
    fn crossing_the_line_finishes_and_everyone_in_ends_the_race() {
        let mut game = Gauntlet::new();
        game.set_bots(2);
        let id = game.connect(&"a".into());
        race(&mut game, COUNTDOWN_SECS + 1.0);
        game.players.get_mut(&id).unwrap().teleport(course().ground(7, 2));
        let events = race(&mut game, 0.1);
        assert!(events.iter().any(|e| matches!(e, Event::Finish { place: 1, .. })));
        assert_eq!(game.phase, Phase::Results);
        assert_eq!(game.players[&id].wins, 1);
        race(&mut game, RESULTS_SECS + 0.1);
        assert_eq!((game.phase, game.played), (Phase::Countdown, 1), "a race counts once its results are over");
        assert!(game.players.values().all(|p| p.finish.is_none() && p.checkpoint == 0 && p.state.frozen));
    }

    #[test]
    fn bot_count_is_settable() {
        let mut game = Gauntlet::new();
        game.set_bots(9);
        game.connect(&"a".into());
        game.tick();
        assert_eq!(game.players.values().filter(|p| p.bot.is_some()).count(), 9);
        game.set_bots(0);
        game.tick();
        assert_eq!(game.players.len(), 1);
    }

    /// Bots alone on the course, no network: they should get through every obstacle and finish.
    fn bots_finish(map: &str) {
        let mut game = Gauntlet::new();
        game.set_map(map);
        let watcher = game.connect(&"observer".into());
        game.players.get_mut(&watcher).unwrap().state.pos = course().spawns[0] - Vec3::X * 2.0;
        let mut falls = 0;
        let events = race(&mut game, COUNTDOWN_SECS + 200.0);
        let mut finishes = vec![];
        for e in &events {
            match e {
                Event::Fell { .. } => falls += 1,
                Event::Finish { id, secs, .. } => finishes.push((game.players[id].name.clone(), *secs)),
                _ => {}
            }
        }
        for p in game.players.values() {
            assert!(in_bounds(p.state.pos), "{} at {:?}", p.name, p.state.pos);
        }
        println!("{falls} falls, finishes: {finishes:?}");
        let stuck: Vec<_> = game.players.values().filter(|p| p.bot.is_some() && p.finish.is_none()).map(|p| (p.checkpoint, p.state.pos)).collect();
        assert!(finishes.len() >= FILL - 2, "{map}: most bots should finish: {finishes:?}, stuck: {stuck:?}");
    }

    #[test]
    fn bots_finish_the_gauntlet() {
        bots_finish("gauntlet");
    }

    #[test]
    fn bots_finish_random_courses() {
        for seed in [1, 2, 3, 4] {
            bots_finish(&format!("random-{seed}"));
        }
    }

    #[test]
    fn random_deals_a_new_course_every_race() {
        let mut game = Gauntlet::new();
        game.set_bots(0);
        let id = game.connect(&"a".into());
        let first = game.map.clone();
        assert!(first.starts_with("random-"), "{first}");
        race(&mut game, COUNTDOWN_SECS + 0.1);
        game.players.get_mut(&id).unwrap().teleport(course().ground(7, 2));
        race(&mut game, RESULTS_SECS + 0.2);
        assert!(game.map.starts_with("random-") && game.map != first, "{} after {first}", game.map);
    }
}

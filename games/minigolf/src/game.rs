//! Minigolf: everyone plays the same hole at once, each at their own pace, balls passing through each other. A hole
//! ends when every person is done (holed or picked up) or at `HOLE_SECS`; then the scorecard and the next hole. After
//! the ninth, the final card and a new course. Bots fill up to `FILL` players.

use crate::bot::Bot;
use crate::map::{self, HOLES, MAPS, course};
use crate::shared::*;
use sdk::{Entity, Frame, GameInfo, ServerGame, log};
use std::collections::BTreeMap;

const TEE_SECS: f32 = 3.0;
const HOLE_SECS: f32 = 150.0;
const CARD_SECS: f32 = 5.0;
const FINAL_SECS: f32 = 10.0;
/// Courses per turn when the host rotates games.
const ROTATE_AFTER: u32 = 1;
/// Players while any human is on, unless the host sets another count: bots make up the rest.
pub const FILL: usize = 4;

type Players = BTreeMap<u32, Player>;

struct Player {
    id: u32,
    ball: Ball,
    card: Vec<u8>,
    done: bool,
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
    /// `color` should be one nobody else has: `sdk::free_color`.
    fn new(id: u32, name: String, color: [u8; 3], bot: Option<Bot>) -> Player {
        Player { id, ball: Ball::default(), card: vec![0; HOLES], done: false, inputs: vec![], teleport: 0, bot, name, color, ping: 0 }
    }

    fn net(&self) -> NetPlayer {
        let (id, ball, done, ping) = (self.id, self.ball, self.done, self.ping);
        NetPlayer { id, ball, card: self.card.clone(), done, bot: self.bot.is_some(), name: self.name.clone(), color: self.color, ping }
    }

    /// Done with the hole, scoring `strokes`.
    fn score(&mut self, hole: usize, strokes: u8) {
        self.done = true;
        self.card[hole] = strokes.clamp(1, MAX_STROKES);
    }
}

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

pub struct Minigolf {
    /// The course being played: `random-<seed>`.
    map: String,
    /// The host picked this course by name: keep it rather than dealing a new one when someone joins.
    picked: bool,
    /// Mixed into the next course's seed: ping times, so courses differ from one server run to the next.
    entropy: u32,
    players: Players,
    tick: u32,
    next_id: u32,
    phase: Phase,
    hole: usize,
    /// When the current phase ends.
    ends: u32,
    events: Vec<Event>,
    /// Players to have, set by the host; None fills up to `FILL`.
    min_players: Option<usize>,
    /// Courses played to the end of their final card since `set_map`, for rotating games.
    played: u32,
}

impl Default for Minigolf {
    fn default() -> Minigolf {
        let mut game = Minigolf {
            map: String::new(),
            picked: false,
            entropy: 0,
            players: Players::new(),
            tick: 0,
            next_id: 1,
            phase: Phase::Tee,
            hole: 0,
            ends: ticks(TEE_SECS),
            events: vec![],
            min_players: None,
            played: 0,
        };
        game.new_course();
        game
    }
}

impl Minigolf {
    pub fn new() -> Minigolf {
        Minigolf::default()
    }
}

impl ServerGame for Minigolf {
    fn connect(&mut self, who: &sdk::Profile) -> u32 {
        let (id, name) = (self.next_id, clean_name(&who.name));
        self.next_id += 1;
        // The first one in gets a fresh course from the first hole rather than wherever the empty server got to.
        if self.players.values().all(|p| p.bot.is_some()) {
            if !self.picked {
                self.new_course();
            }
            self.restart();
        }
        log(&format!("{name} (player {id}) joined"));
        let mut p = Player::new(id, name, sdk::free_color(self.players.values().map(|p| p.color)), None);
        self.join(&mut p);
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
        self.update_round();
        let entities: Vec<Entity> = self.players.values().map(|p| p.net().entity(p.teleport)).collect();
        let time = self.ends.saturating_sub(self.tick) as f32 / TICK_HZ as f32;
        let state = Round { phase: self.phase, hole: self.hole as u8, time };
        let events = self.events.drain(..).map(|e| sdk::Event { to: None, data: encode(&e) }).collect();
        Frame { map: self.map.clone(), state: encode(&state), entities, events, finished: self.played >= ROTATE_AFTER }
    }

    fn info(&mut self) -> GameInfo {
        let bots = self.players.values().filter(|p| p.bot.is_some()).count() as u32;
        let maps = MAPS.map(String::from).to_vec();
        GameInfo { game: "Minigolf".into(), map: self.map.clone(), maps, players: self.players.len() as u32 - bots, bots }
    }

    fn set_map(&mut self, map: &str) {
        if map != "random" && !map.starts_with("random-") {
            return log(&format!("no map called {map}; playing {}", self.map));
        }
        self.played = 0;
        self.picked = map.starts_with("random-") && map::load(map);
        if self.picked {
            self.map = map.into();
        } else {
            self.new_course();
        }
        self.restart();
    }

    fn set_min_players(&mut self, players: u32) {
        log(&format!("bots fill up to {players} players"));
        self.min_players = Some(players as usize);
    }
}

impl Minigolf {
    fn new_course(&mut self) {
        self.entropy = (self.entropy ^ self.tick).wrapping_mul(0x9E37_79B9).rotate_left(13);
        self.map = format!("random-{}", self.entropy % 100_000);
        map::load(&self.map);
        log(&format!("course {}", self.map));
    }

    /// To the tee of the current hole, frozen unless it's being played. Someone joining after the first hole scores
    /// the most on the holes they missed, and if the hole is already over, on this one too.
    fn join(&self, p: &mut Player) {
        p.card = (0..HOLES).map(|h| if h < self.hole { MAX_STROKES } else { 0 }).collect();
        p.done = false;
        if matches!(self.phase, Phase::Card | Phase::Final) {
            p.score(self.hole, MAX_STROKES);
        }
        self.to_tee(p);
    }

    fn to_tee(&self, p: &mut Player) {
        p.ball = Ball::on_tee(course().holes[self.hole].tee, self.phase != Phase::Play);
        p.teleport = p.teleport.wrapping_add(1);
    }

    /// The first hole of the course, from the tee.
    fn restart(&mut self) {
        (self.phase, self.hole, self.ends) = (Phase::Tee, 0, self.tick + ticks(TEE_SECS));
        self.next_hole_for_all(true);
    }

    fn next_hole_for_all(&mut self, fresh_card: bool) {
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for id in ids {
            let mut p = self.players.remove(&id).unwrap();
            if fresh_card {
                p.card = vec![0; HOLES];
            }
            p.done = false;
            self.to_tee(&mut p);
            self.players.insert(id, p);
        }
    }

    fn update_round(&mut self) {
        if self.players.is_empty() {
            if self.phase != Phase::Tee || self.hole != 0 {
                self.restart();
            }
            self.ends = self.tick + ticks(TEE_SECS);
            return;
        }
        let humans_done = self.players.values().filter(|p| p.bot.is_none()).all(|p| p.done);
        match self.phase {
            Phase::Tee if self.tick >= self.ends => {
                (self.phase, self.ends) = (Phase::Play, self.tick + ticks(HOLE_SECS));
                self.players.values_mut().for_each(|p| p.ball.frozen = false);
                self.events.push(Event::Go);
            }
            Phase::Play if self.tick >= self.ends || humans_done => {
                let last = self.hole + 1 == HOLES;
                (self.phase, self.ends) = if last { (Phase::Final, self.tick + ticks(FINAL_SECS)) } else { (Phase::Card, self.tick + ticks(CARD_SECS)) };
                let hole = self.hole;
                for p in self.players.values_mut().filter(|p| !p.done) {
                    p.score(hole, MAX_STROKES);
                }
                self.players.values_mut().for_each(|p| p.ball.frozen = true);
                if last && let Some(w) = self.players.values().min_by_key(|p| p.net().total()) {
                    log(&format!("{} wins with {}", w.name, w.net().total()));
                }
            }
            Phase::Card if self.tick >= self.ends => {
                (self.phase, self.hole, self.ends) = (Phase::Tee, self.hole + 1, self.tick + ticks(TEE_SECS));
                self.next_hole_for_all(false);
            }
            Phase::Final if self.tick >= self.ends => {
                (self.played, self.picked) = (self.played + 1, false);
                self.new_course();
                self.restart();
            }
            _ => {}
        }
    }

    /// One tick of play: bots decide, then every player's inputs since the last tick are applied to their ball.
    fn run_inputs(&mut self) {
        let hole = self.hole;
        for p in self.players.values_mut() {
            if let Some(bot) = &mut p.bot {
                let inp = if p.done { Input::default() } else { bot.think(&p.ball) };
                p.inputs.push(inp);
            }
        }
        for p in self.players.values_mut() {
            for inp in std::mem::take(&mut p.inputs) {
                let step = step_ball(&mut p.ball, &inp);
                if step.holed && !p.done {
                    p.score(hole, p.ball.strokes);
                    self.events.push(Event::Holed { id: p.id, strokes: p.ball.strokes });
                    log(&format!("{} holes out in {}", p.name, p.ball.strokes));
                }
                if let Some(water) = step.out {
                    self.events.push(Event::Out { id: p.id, at: p.ball.pos, water });
                }
                if step.back {
                    p.teleport = p.teleport.wrapping_add(1);
                }
            }
            if !p.done && p.ball.at_rest() && p.ball.strokes >= MAX_STROKES {
                p.score(hole, MAX_STROKES);
                self.events.push(Event::PickedUp { id: p.id });
            }
        }
    }

    /// Adds or removes bots so there are `min_players` players (default `FILL`) while any human is on, and no bots
    /// without.
    fn fill_bots(&mut self) {
        let humans = self.players.values().filter(|p| p.bot.is_none()).count();
        let want = if humans == 0 { 0 } else { self.min_players.unwrap_or(FILL).saturating_sub(humans) };
        let mut bots: Vec<u32> = self.players.values().filter(|p| p.bot.is_some()).map(|p| p.id).collect();
        while bots.len() > want {
            self.players.remove(&bots.pop().unwrap());
        }
        for _ in bots.len()..want {
            let id = self.next_id;
            self.next_id += 1;
            let mut p = Player::new(id, bot_name(id), sdk::free_color(self.players.values().map(|p| p.color)), Some(Bot::new(id)));
            self.join(&mut p);
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
    const NAMES: [&str; 12] = ["Birdie", "Bogey", "Caddie", "Chip", "Divot", "Eagle", "Fore", "Links", "Putter", "Sandy", "Tee", "Wedge"];
    format!("BOT {}", NAMES[id as usize % NAMES.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the game, people sending `inp` every tick.
    fn play(game: &mut Minigolf, secs: f32, inp: Input) -> Vec<Event> {
        let humans: Vec<u32> = game.players.values().filter(|p| p.bot.is_none()).map(|p| p.id).collect();
        let mut events = vec![];
        for _ in 0..ticks(secs) {
            humans.iter().for_each(|&id| game.input(id, 0, 0.0, &encode(&inp)));
            events.extend(game.tick().events.iter().filter_map(|e| decode::<Event>(&e.data)));
        }
        events
    }

    #[test]
    fn frozen_on_the_tee_until_go() {
        let mut game = Minigolf::new();
        game.set_min_players(0);
        let id = game.connect(&"a".into());
        game.input(id, 1, 0.0, &encode(&Input { shot: 0.5, ..Default::default() }));
        game.tick();
        assert_eq!(game.players[&id].ball.strokes, 0);
        assert!(play(&mut game, TEE_SECS, Input::default()).iter().any(|e| matches!(e, Event::Go)));
        play(&mut game, 0.01, Input { shot: 0.5, ..Default::default() });
        assert_eq!(game.players[&id].ball.strokes, 1);
    }

    #[test]
    fn holing_out_ends_the_hole_and_moves_everyone_on() {
        let mut game = Minigolf::new();
        game.set_min_players(3);
        let id = game.connect(&"a".into());
        play(&mut game, TEE_SECS + 0.1, Input::default());
        let cup = course().holes[0].cup;
        let p = game.players.get_mut(&id).unwrap();
        (p.ball.pos, p.ball.vel, p.ball.slow) = (cup + glam::Vec3::new(0.3, BALL_RADIUS, 0.0), glam::Vec3::X * -1.0, 0);
        p.ball.strokes = 2;
        let events = play(&mut game, 1.0, Input::default());
        assert!(events.iter().any(|e| matches!(e, Event::Holed { strokes: 2, .. })), "{events:?}");
        assert_eq!(game.phase, Phase::Card, "the only person is done");
        assert_eq!(game.players[&id].card[0], 2);
        assert!(game.players.values().all(|p| p.card[0] > 0), "bots still out score the most");
        play(&mut game, CARD_SECS, Input::default());
        assert_eq!((game.phase, game.hole), (Phase::Tee, 1));
        let tee = course().holes[1].tee;
        assert!(game.players.values().all(|p| p.ball.pos.distance(tee) < 0.5 && !p.done && p.ball.frozen));
    }

    #[test]
    fn a_whole_course_then_a_new_one() {
        let mut game = Minigolf::new();
        game.set_min_players(0);
        game.connect(&"a".into());
        let first = game.map.clone();
        for h in 0..HOLES {
            play(&mut game, TEE_SECS + HOLE_SECS + 0.05, Input::default());
            if h + 1 < HOLES {
                play(&mut game, CARD_SECS, Input::default());
            }
        }
        assert_eq!(game.phase, Phase::Final);
        assert!(game.players.values().all(|p| p.net().total() == (HOLES as u32) * MAX_STROKES as u32));
        play(&mut game, FINAL_SECS + 0.1, Input::default());
        assert_eq!((game.phase, game.hole, game.played), (Phase::Tee, 0, 1));
        assert_ne!(game.map, first);
        assert!(game.players.values().all(|p| p.net().total() == 0));
    }

    #[test]
    fn joining_late_scores_the_holes_missed() {
        let mut game = Minigolf::new();
        game.set_min_players(0);
        game.connect(&"a".into());
        play(&mut game, TEE_SECS + HOLE_SECS + CARD_SECS + 0.2, Input::default());
        assert_eq!(game.hole, 1);
        let b = game.connect(&"b".into());
        assert_eq!(game.players[&b].card[0], MAX_STROKES);
        assert_eq!(game.players[&b].card[1], 0);
    }

    #[test]
    fn bots_fill_up_to_the_player_count() {
        let mut game = Minigolf::new();
        game.set_min_players(6);
        game.connect(&"a".into());
        game.tick();
        assert_eq!(game.players.values().filter(|p| p.bot.is_some()).count(), 5);
        game.set_min_players(0);
        game.tick();
        assert_eq!(game.players.len(), 1);
    }
}

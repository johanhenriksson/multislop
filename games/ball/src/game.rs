//! Ballslop: two teams of four, a giant ball and rocket launchers. Rockets blast the ball and running into it pushes
//! it; most goals in 5 minutes wins, a tie goes to golden goal. Bots fill both teams while anyone is playing.

use crate::bot::{Bot, Role, Seen, View};
use crate::map::{GOAL_LINE, MAP, MAPS};
use crate::shared::*;
use glam::Vec3;
use sdk::{Frame, GameInfo, ServerGame, log};
use std::collections::BTreeMap;

/// Players per team while any human is on, unless the host sets a bot count: bots make up the rest.
pub const TEAM_SIZE: usize = 4;
const MATCH_SECS: f32 = 300.0;
const OVERTIME_SECS: f32 = 120.0;
const KICKOFF_SECS: f32 = 3.0;
const GOAL_SECS: f32 = 3.0;
const END_SECS: f32 = 8.0;
/// Matches per turn when the host rotates games.
const ROTATE_AFTER: u32 = 2;
pub const RESPAWN_SECS: f32 = 7.0;
/// How near a pickup's spot you have to walk to take it.
const PICKUP_REACH: f32 = 1.2;

type Players = BTreeMap<u32, Player>;

struct Player {
    /// What clients see; the server keeps everything else beside it.
    net: NetPlayer,
    died_at: u32,
    /// Inputs received since the last tick, applied in order on the next one.
    inputs: Vec<Input>,
    /// Bumped on every respawn and kickoff; see `sdk::Entity::teleport`.
    teleport: u8,
    bot: Option<Bot>,
}

impl Player {
    fn new(id: u32, team: Team, name: String, bot: Option<Bot>) -> Player {
        let net = NetPlayer {
            id,
            team,
            state: PlayerState::default(),
            hp: 0,
            armor: 0,
            alive: false,
            goals: 0,
            kills: 0,
            deaths: 0,
            bot: bot.is_some(),
            name,
            ping: 0,
        };
        Player { net, died_at: 0, inputs: vec![], teleport: 0, bot }
    }

    /// Back at `pos` with full health, no armor and a fresh load, facing the center spot.
    fn spawn(&mut self, pos: Vec3, frozen: bool) {
        self.net.state = PlayerState::spawn(pos, f32::atan2(pos.x, pos.z), frozen);
        (self.net.hp, self.net.armor, self.net.alive) = (MAX_HP, 0, true);
        self.teleport = self.teleport.wrapping_add(1);
    }
}

struct Rocket {
    id: u32,
    owner: u32,
    team: Team,
    pos: Vec3,
    vel: Vec3,
    /// Goes off on its own at this tick.
    fuse: u32,
}

/// What teammates, the shooter included, take of a blast's damage.
const FRIENDLY_DAMAGE: f32 = 0.5;

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

pub struct Ballslop {
    map: String,
    players: Players,
    rockets: Vec<Rocket>,
    tick: u32,
    next_id: u32,
    next_rocket: u32,
    ball: Vec3,
    ball_vel: Vec3,
    /// Bumped whenever the ball is put back on the center spot.
    ball_teleport: u8,
    /// The last player of each team to touch the ball since kickoff, for goal credit.
    touched: [Option<u32>; 2],
    last_touch: Option<Team>,
    score: [u32; 2],
    phase: Phase,
    /// The kickoff countdown, goal or end screen is over at this tick.
    phase_ends: u32,
    /// Ticks of play left on the clock.
    clock: u32,
    overtime: bool,
    /// The tick each of `MAP.pickups` is back at; it's up once that has passed.
    pickups: Vec<u32>,
    events: Vec<Event>,
    /// Bots to play with, set by the host; None fills each team up to `TEAM_SIZE`.
    bots: Option<usize>,
    /// Matches played to the end of their scoreboard since `set_map`, for rotating games.
    played: u32,
}

impl Default for Ballslop {
    fn default() -> Ballslop {
        let mut game = Ballslop {
            map: MAPS[0].into(),
            players: Players::new(),
            rockets: vec![],
            tick: 0,
            next_id: 1,
            next_rocket: 0,
            ball: Vec3::ZERO,
            ball_vel: Vec3::ZERO,
            ball_teleport: 0,
            touched: [None; 2],
            last_touch: None,
            score: [0; 2],
            phase: Phase::Play,
            phase_ends: 0,
            clock: 0,
            overtime: false,
            pickups: vec![],
            events: vec![],
            bots: None,
            played: 0,
        };
        game.new_match();
        game
    }
}

impl Ballslop {
    pub fn new() -> Ballslop {
        Ballslop::default()
    }
}

impl ServerGame for Ballslop {
    fn connect(&mut self, name: &str) -> u32 {
        let (id, name) = (self.next_id, clean_name(name));
        self.next_id += 1;
        // Onto the team with fewer people, then fewer players, Orange on a tie.
        let count = |team: Team, humans: bool| self.players.values().filter(|p| p.net.team == team && (!humans || p.bot.is_none())).count();
        let key = |t: Team| (count(t, true), count(t, false));
        let team = if key(Team::Blue) < key(Team::Orange) { Team::Blue } else { Team::Orange };
        log(&format!("{name} (player {id}) joined {team:?}"));
        let mut p = Player::new(id, team, name, None);
        p.spawn(respawn_spot(&self.players, team, self.tick), self.phase != Phase::Play);
        self.players.insert(id, p);
        id
    }

    fn disconnect(&mut self, id: u32) {
        if let Some(p) = self.players.remove(&id) {
            log(&format!("{} left", p.net.name));
        }
    }

    fn ping(&mut self, id: u32, ms: u16) {
        if let Some(p) = self.players.get_mut(&id) {
            p.net.ping = ms;
        }
    }

    fn input(&mut self, id: u32, seq: u32, _view_tick: f32, input: &[u8]) {
        if let (Some(p), Some(inp)) = (self.players.get_mut(&id), decode::<Input>(input)) {
            p.inputs.push(Input { seq, ..inp });
        }
    }

    fn tick(&mut self) -> Frame {
        fill_bots(&mut self.players, &mut self.next_id, self.tick, self.bots);
        let frozen = self.phase != Phase::Play;
        for p in self.players.values_mut() {
            p.net.state.frozen = frozen;
        }
        self.run_inputs();
        self.update_rockets();
        if !matches!(self.phase, Phase::Kickoff { .. }) {
            self.update_ball();
        }
        self.update_players();
        self.tick += 1;
        self.update_match();

        let mut entities: Vec<Entity> = self.players.values().map(|p| p.net.entity(p.teleport)).collect();
        entities.push(Entity { id: BALL_ID, pos: self.ball, vel: self.ball_vel, teleport: self.ball_teleport, data: encode(&Thing::Ball), ..Default::default() });
        entities.extend(self.rockets.iter().map(|r| Entity {
            id: r.id,
            pos: r.pos,
            vel: r.vel,
            data: encode(&Thing::Rocket(NetRocket { owner: r.owner, team: r.team })),
            ..Default::default()
        }));
        let state = Match {
            score: self.score,
            phase: match self.phase {
                Phase::Kickoff { .. } => Phase::Kickoff { countdown: self.phase_ends.saturating_sub(self.tick) as f32 / TICK_HZ as f32 },
                phase => phase,
            },
            time_left: self.clock as f32 / TICK_HZ as f32,
            overtime: self.overtime,
            pickups: self.pickups.iter().map(|&back| self.tick >= back).collect(),
            last_touch: self.last_touch,
        };
        let events = self.events.drain(..).map(|e| sdk::Event { to: None, data: encode(&e) }).collect();
        Frame { map: self.map.clone(), state: encode(&state), entities, events, finished: self.played >= ROTATE_AFTER }
    }

    fn info(&mut self) -> GameInfo {
        let bots = self.players.values().filter(|p| p.bot.is_some()).count() as u32;
        let maps = MAPS.map(String::from).to_vec();
        GameInfo { game: "Ballslop".into(), map: self.map.clone(), maps, players: self.players.len() as u32 - bots, bots }
    }

    fn set_map(&mut self, map: &str) {
        if !MAPS.contains(&map) {
            return log(&format!("no map called {map}; playing {}", self.map));
        }
        (self.map, self.played) = (map.into(), 0);
        self.new_match();
    }

    fn set_bots(&mut self, bots: u32) {
        log(&format!("{bots} bots"));
        self.bots = Some(bots as usize);
    }
}

impl Ballslop {
    /// Scores, stats, clock and pickups back to the start, and kickoff.
    fn new_match(&mut self) {
        (self.score, self.clock, self.overtime) = ([0; 2], ticks(MATCH_SECS), false);
        self.pickups = vec![0; MAP.pickups.len()];
        for p in self.players.values_mut() {
            (p.net.goals, p.net.kills, p.net.deaths, p.net.alive) = (0, 0, 0, false);
        }
        self.kickoff();
    }

    /// Everyone on their kickoff spot (the dead come back fresh), the ball on the center spot, and the countdown.
    fn kickoff(&mut self) {
        (self.phase, self.phase_ends) = (Phase::Kickoff { countdown: KICKOFF_SECS }, self.tick + ticks(KICKOFF_SECS));
        (self.ball, self.ball_vel, self.ball_teleport) = (Vec3::Y * BALL_RADIUS, Vec3::ZERO, self.ball_teleport.wrapping_add(1));
        (self.touched, self.last_touch) = ([None; 2], None);
        self.rockets.clear();
        for team in [Team::Orange, Team::Blue] {
            let spots = MAP.kickoffs[team as usize].iter().chain(MAP.spawns[team as usize].iter().cycle());
            for (p, &pos) in self.players.values_mut().filter(|p| p.net.team == team).zip(spots) {
                if p.net.alive {
                    let s = &mut p.net.state;
                    (s.pos, s.vel, s.yaw, s.frozen) = (pos, Vec3::ZERO, f32::atan2(pos.x, pos.z), true);
                    p.teleport = p.teleport.wrapping_add(1);
                } else {
                    p.spawn(pos, true);
                }
            }
        }
    }

    /// One tick of play: bots decide, then every player's inputs since the last tick are applied and rockets leave.
    fn run_inputs(&mut self) {
        let seen: Vec<Seen> = self.players.values().map(|p| Seen { id: p.net.id, team: p.net.team, pos: p.net.state.pos, vel: p.net.state.vel, alive: p.net.alive }).collect();
        let roles = roles(&self.players, self.ball);
        let view = View { ball: self.ball, ball_vel: self.ball_vel, players: &seen, pickups: &self.pickups.iter().map(|&back| self.tick >= back).collect::<Vec<_>>() };
        for p in self.players.values_mut() {
            if let Some(bot) = &mut p.bot {
                let role = roles.get(&p.net.id).copied().unwrap_or(Role::Support);
                p.inputs.push(bot.think(&p.net, role, &view, self.tick));
            }
        }
        for p in self.players.values_mut() {
            for inp in std::mem::take(&mut p.inputs) {
                if !p.net.alive {
                    continue;
                }
                let Some(dir) = step_player(&mut p.net.state, &inp).fired else { continue };
                let from = eye(&p.net.state);
                let id = ROCKET_IDS | (self.next_rocket % ROCKET_IDS);
                self.next_rocket = self.next_rocket.wrapping_add(1);
                let fuse = self.tick + ticks(ROCKET_FUSE);
                self.rockets.push(Rocket { id, owner: p.net.id, team: p.net.team, pos: from, vel: dir * ROCKET_SPEED, fuse });
                self.events.push(Event::Launch { shooter: p.net.id, from, dir });
            }
        }
    }

    /// Flies rockets, setting off those that touch the world, anyone but the shooter, or the ball, or whose fuse has
    /// burnt.
    fn update_rockets(&mut self) {
        let mut blasts = vec![];
        self.rockets.retain_mut(|r| {
            let (len, dir) = (r.vel.length() * DT, r.vel.normalize_or_zero());
            let others = self.players.values().filter(|p| p.net.alive && p.net.id != r.owner);
            let hit = others
                .filter_map(|p| trace_capsule(r.pos, dir, p.net.state.pos))
                .chain(trace_ball(r.pos, dir, self.ball))
                .fold(trace_world(r.pos, dir).0, f32::min);
            if hit > len && self.tick < r.fuse {
                r.pos += dir * len;
                return true;
            }
            // Just short of the surface, so the blast's line of sight doesn't start inside it.
            blasts.push((r.pos + dir * (hit.min(len) - 0.05).max(0.0), r.owner, r.team));
            false
        });
        for (at, owner, team) in blasts {
            self.explode(at, owner, team);
        }
    }

    /// Damages everyone in the open within the blast radius (teammates and the shooter less), throws them, and
    /// pushes the ball.
    fn explode(&mut self, at: Vec3, owner: u32, team: Team) {
        self.events.push(Event::Explosion { pos: at, radius: BLAST_RADIUS, team });
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for id in ids {
            let p = self.players.get_mut(&id).unwrap();
            let dist = body(p.net.state.pos).distance(at);
            if !p.net.alive || dist >= BLAST_RADIUS || !can_see(at, body(p.net.state.pos)) {
                continue;
            }
            let push = knockback(p.net.state.pos, at);
            p.net.state.vel += push;
            p.net.state.on_ground &= push.y <= 0.0;
            let full = BLAST_DAMAGE * (1.0 - dist / BLAST_RADIUS);
            let damage = if id == owner || p.net.team == team { full * FRIENDLY_DAMAGE } else { full };
            self.hurt(owner, id, damage);
        }
        if let Some(strength) = blast_ball(self.ball, &mut self.ball_vel, at) {
            self.touch(owner, team);
            self.events.push(Event::BallHit { pos: self.ball, strength, kind: HitKind::Blast });
        }
    }

    /// Armor takes its share until it runs out; at 0 HP the victim dies, and the attacker gets a kill for an enemy and
    /// loses one for a teammate.
    fn hurt(&mut self, attacker: u32, victim: u32, damage: f32) {
        let v = &mut self.players.get_mut(&victim).unwrap().net;
        let absorbed = ((damage * ARMOR_TAKES) as i32).min(v.armor);
        v.armor -= absorbed;
        v.hp -= (damage.round() as i32 - absorbed).max(1);
        if v.hp > 0 {
            return;
        }
        (v.hp, v.alive, v.deaths) = (0, false, v.deaths + 1);
        let team = v.team;
        self.players.get_mut(&victim).unwrap().died_at = self.tick;
        if attacker != victim
            && let Some(k) = self.players.get_mut(&attacker)
        {
            k.net.kills += if k.net.team == team { -1 } else { 1 };
        }
        self.events.push(Event::Kill { killer: attacker, victim });
    }

    fn touch(&mut self, id: u32, team: Team) {
        self.touched[team as usize] = Some(id);
        self.last_touch = Some(team);
    }

    /// Players push the ball out of their way, then it moves; a ball that got lost goes back to the center spot.
    fn update_ball(&mut self) {
        let mut touches = vec![];
        for p in self.players.values().filter(|p| p.net.alive) {
            if let Some(strength) = touch_ball(&mut self.ball, &mut self.ball_vel, p.net.state.pos, p.net.state.vel) {
                touches.push((p.net.id, p.net.team, strength));
            }
        }
        for (id, team, strength) in touches {
            self.touch(id, team);
            self.events.push(Event::BallHit { pos: self.ball, strength, kind: HitKind::Push });
        }
        step_ball(&mut self.ball, &mut self.ball_vel);
        if !in_bounds(self.ball) || !self.ball_vel.is_finite() {
            log(&format!("ball lost at {}, back to the center spot", self.ball));
            (self.ball, self.ball_vel, self.ball_teleport) = (Vec3::Y * BALL_RADIUS, Vec3::ZERO, self.ball_teleport.wrapping_add(1));
        }
    }

    /// Pickups taken by whoever walks over them and needs them, mega health draining, and the dead coming back.
    fn update_players(&mut self) {
        for (i, &(at, item)) in MAP.pickups.iter().enumerate() {
            if self.tick < self.pickups[i] {
                continue;
            }
            let near = |p: &&mut Player| {
                let d = p.net.state.pos - at;
                p.net.alive && d.x.abs() < PICKUP_REACH && d.z.abs() < PICKUP_REACH && d.y.abs() < 1.0
            };
            if let Some(p) = self.players.values_mut().filter(near).find_map(|p| take(&mut p.net, item).then_some(p.net.id)) {
                self.pickups[i] = self.tick + ticks(item.respawn_secs());
                self.events.push(Event::Pickup { player: p, kind: item });
            }
        }
        let drain = self.tick.is_multiple_of(TICK_HZ as u32);
        for p in self.players.values_mut().filter(|p| p.net.alive && drain && p.net.hp > MAX_HP) {
            p.net.hp -= 1;
        }
        let back: Vec<(u32, Team)> = self.players.values().filter(|p| !p.net.alive && self.tick >= p.died_at + ticks(RESPAWN_SECS)).map(|p| (p.net.id, p.net.team)).collect();
        for (id, team) in back.into_iter().filter(|_| self.phase == Phase::Play) {
            let pos = respawn_spot(&self.players, team, self.tick);
            self.players.get_mut(&id).unwrap().spawn(pos, false);
        }
    }

    /// Goals, the clock, overtime, and moving from kickoff to play to goal to the end screen and the next match.
    fn update_match(&mut self) {
        if self.players.is_empty() {
            return self.new_match();
        }
        match self.phase {
            Phase::Kickoff { .. } if self.tick >= self.phase_ends => self.phase = Phase::Play,
            Phase::Play if self.ball.z.abs() > GOAL_LINE + BALL_RADIUS => self.goal(),
            Phase::Play => {
                self.clock = self.clock.saturating_sub(1);
                if self.clock > 0 {
                    return;
                }
                if self.score[0] == self.score[1] && !self.overtime {
                    log("tied: golden goal");
                    (self.overtime, self.clock) = (true, ticks(OVERTIME_SECS));
                    self.kickoff();
                } else {
                    self.end();
                }
            }
            Phase::Goal { .. } if self.tick >= self.phase_ends => {
                if self.overtime { self.end() } else { self.kickoff() }
            }
            Phase::End { .. } if self.tick >= self.phase_ends => {
                self.played += 1;
                self.new_match();
            }
            _ => {}
        }
    }

    /// The ball is in a goal: the point goes to the other team, credited to its last toucher, or the defenders' last
    /// one as an own goal if only they touched it.
    fn goal(&mut self) {
        let defending = if self.ball.z > 0.0 { Team::Orange } else { Team::Blue };
        let team = defending.other();
        let (scorer, own_goal) = match (self.touched[team as usize], self.touched[defending as usize]) {
            (Some(id), _) => (Some(id), false),
            (None, Some(id)) => (Some(id), true),
            (None, None) => (None, false),
        };
        self.score[team as usize] += 1;
        if let Some(p) = scorer.filter(|_| !own_goal).and_then(|id| self.players.get_mut(&id)) {
            p.net.goals += 1;
        }
        let name = scorer.and_then(|id| self.players.get(&id)).map_or("nobody", |p| &p.net.name);
        log(&format!("{team:?} scores ({name}{}): {} - {}", if own_goal { ", own goal" } else { "" }, self.score[0], self.score[1]));
        self.events.push(Event::Goal { team, scorer, own_goal });
        (self.phase, self.phase_ends) = (Phase::Goal { team, scorer, own_goal }, self.tick + ticks(GOAL_SECS));
    }

    fn end(&mut self) {
        let winner = match self.score[0].cmp(&self.score[1]) {
            std::cmp::Ordering::Greater => Some(Team::Orange),
            std::cmp::Ordering::Less => Some(Team::Blue),
            std::cmp::Ordering::Equal => None,
        };
        log(&format!("match over: {winner:?} wins, {} - {}", self.score[0], self.score[1]));
        (self.phase, self.phase_ends) = (Phase::End { winner }, self.tick + ticks(END_SECS));
    }
}

/// Gives `item` to a player who's better off for it. Returns whether they took it.
fn take(p: &mut NetPlayer, item: Item) -> bool {
    let (stat, add, most) = match item {
        Item::Health => (&mut p.hp, 25, MAX_HP),
        Item::Mega => (&mut p.hp, 100, MEGA_HP),
        Item::Armor => (&mut p.armor, 50, MAX_ARMOR),
        Item::HeavyArmor => (&mut p.armor, 100, HEAVY_ARMOR),
        Item::Rockets => {
            let reserve = &mut p.state.ammo.1;
            let useful = *reserve < MAX_RESERVE;
            *reserve = (*reserve + 8).min(MAX_RESERVE);
            return useful;
        }
    };
    let useful = *stat < most;
    *stat = (*stat + add).min(most).max(*stat);
    useful
}

/// Who plays which part per team, by the ball: the nearest attacks, the one furthest back keeps, the rest support.
fn roles(players: &Players, ball: Vec3) -> BTreeMap<u32, Role> {
    let mut roles = BTreeMap::new();
    for team in [Team::Orange, Team::Blue] {
        let mut mates: Vec<&NetPlayer> = players.values().map(|p| &p.net).filter(|p| p.alive && p.team == team).collect();
        mates.sort_by(|a, b| a.state.pos.distance(ball).total_cmp(&b.state.pos.distance(ball)));
        let Some((attacker, rest)) = mates.split_first() else { continue };
        roles.insert(attacker.id, Role::Attack);
        let keeper = rest.iter().max_by(|a, b| (a.state.pos.z * team.end()).total_cmp(&(b.state.pos.z * team.end())));
        for p in rest {
            roles.insert(p.id, if keeper.is_some_and(|k| k.id == p.id) { Role::Keep } else { Role::Support });
        }
    }
    roles
}

/// A respawn spot of `team` with nobody standing on it, starting from a different one each time.
fn respawn_spot(players: &Players, team: Team, tick: u32) -> Vec3 {
    let spots = &MAP.spawns[team as usize];
    let free = |s: &&Vec3| players.values().all(|p| !p.net.alive || p.net.state.pos.distance(**s) > 2.0);
    let start = tick as usize % spots.len();
    *spots.iter().cycle().skip(start).take(spots.len()).find(free).unwrap_or(&spots[start])
}

/// Printable, at most 16 characters, never empty.
fn clean_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(16).collect();
    if name.trim().is_empty() { "player".into() } else { name.trim().into() }
}

fn bot_name(id: u32) -> String {
    const NAMES: [&str; 12] = ["Bergkamp", "Cantona", "Cruyff", "Eusebio", "Garrincha", "Kopa", "Maldini", "Pele", "Puskas", "Sindelar", "Yashin", "Zidane"];
    format!("BOT {}", NAMES[id as usize % NAMES.len()])
}

/// Bots wanted per team: none without a human; else enough to make `TEAM_SIZE` a side, or the host's count split so
/// the teams come out as even as they can.
fn bot_targets(players: &Players, bots: Option<usize>) -> [usize; 2] {
    let humans = |t: Team| players.values().filter(|p| p.bot.is_none() && p.net.team == t).count();
    let humans = [humans(Team::Orange), humans(Team::Blue)];
    if humans == [0, 0] {
        return [0, 0];
    }
    let Some(n) = bots else { return humans.map(|h| TEAM_SIZE.saturating_sub(h)) };
    let mut want = [0, 0];
    for _ in 0..n {
        want[(humans[1] + want[1] < humans[0] + want[0]) as usize] += 1;
    }
    want
}

/// Adds or removes bots on each team to meet `bot_targets`.
fn fill_bots(players: &mut Players, next_id: &mut u32, tick: u32, bots: Option<usize>) {
    let want = bot_targets(players, bots);
    for team in [Team::Orange, Team::Blue] {
        let mut have: Vec<u32> = players.values().filter(|p| p.bot.is_some() && p.net.team == team).map(|p| p.net.id).collect();
        while have.len() > want[team as usize] {
            players.remove(&have.pop().unwrap());
        }
        for _ in have.len()..want[team as usize] {
            let id = *next_id;
            *next_id += 1;
            let mut p = Player::new(id, team, bot_name(id), Some(Bot::new(id)));
            let frozen = players.values().any(|p| p.net.state.frozen);
            p.spawn(respawn_spot(players, team, tick + id), frozen);
            log(&format!("{} joined {team:?}", p.net.name));
            players.insert(id, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(game: &mut Ballslop, id: u32, pos: Vec3) {
        let s = &mut game.players.get_mut(&id).unwrap().net.state;
        (s.pos, s.vel) = (pos, Vec3::ZERO);
    }

    /// Two humans, one a side, no bots, in play.
    fn duel() -> (Ballslop, u32, u32) {
        let mut game = Ballslop::new();
        game.set_bots(0);
        let (a, b) = (game.connect("a"), game.connect("b"));
        game.phase = Phase::Play;
        (game, a, b)
    }

    #[test]
    fn teams_balance_and_bots_fill_each_side() {
        let mut game = Ballslop::new();
        let a = game.connect("a");
        game.tick();
        let team = |game: &Ballslop, t: Team| game.players.values().filter(|p| p.net.team == t).count();
        assert_eq!((team(&game, Team::Orange), team(&game, Team::Blue)), (TEAM_SIZE, TEAM_SIZE));
        let b = game.connect("b");
        assert_ne!(game.players[&a].net.team, game.players[&b].net.team, "humans split");
        game.set_bots(3);
        game.tick();
        assert_eq!((team(&game, Team::Orange), team(&game, Team::Blue)), (3, 2));
        game.set_bots(0);
        game.tick();
        assert_eq!(game.players.len(), 2);
    }

    #[test]
    fn a_goal_scores_and_kickoff_follows() {
        let (mut game, a, b) = duel();
        let (team_a, team_b) = (game.players[&a].net.team, game.players[&b].net.team);
        // a shoved it into b's goal.
        game.touch(a, team_a);
        (game.ball, game.ball_vel) = (Vec3::new(0.0, BALL_RADIUS, team_b.end() * (GOAL_LINE + 1.5)), Vec3::Z * team_b.end() * 5.0);
        let teleport = game.ball_teleport;
        let events = game.tick().events;
        assert!(events.iter().any(|e| decode::<Event>(&e.data) == Some(Event::Goal { team: team_a, scorer: Some(a), own_goal: false })));
        assert_eq!((game.score[team_a as usize], game.players[&a].net.goals), (1, 1));
        assert!(matches!(game.phase, Phase::Goal { .. }));
        for _ in 0..ticks(GOAL_SECS) {
            game.tick();
        }
        assert!(matches!(game.phase, Phase::Kickoff { .. }), "{:?}", game.phase);
        assert!(game.ball == Vec3::Y * BALL_RADIUS && game.ball_teleport != teleport);
        assert!(game.players.values().all(|p| MAP.kickoffs[p.net.team as usize].contains(&p.net.state.pos)));
        // Now only b touches it, into b's own goal.
        for _ in 0..ticks(KICKOFF_SECS) + 1 {
            game.tick();
        }
        assert_eq!(game.phase, Phase::Play);
        game.touch(b, team_b);
        game.ball = Vec3::new(0.0, BALL_RADIUS, team_b.end() * (GOAL_LINE + 2.0));
        game.tick();
        assert_eq!(game.phase, Phase::Goal { team: team_a, scorer: Some(b), own_goal: true });
        assert_eq!((game.score[team_a as usize], game.players[&b].net.goals), (2, 0));
    }

    #[test]
    fn a_tie_goes_to_golden_goal_then_a_draw() {
        let (mut game, _, _) = duel();
        game.clock = 1;
        game.tick();
        assert!(game.overtime && matches!(game.phase, Phase::Kickoff { .. }) && game.clock == ticks(OVERTIME_SECS));
        game.phase = Phase::Play;
        game.clock = 1;
        game.tick();
        assert_eq!(game.phase, Phase::End { winner: None });
        for _ in 0..ticks(END_SECS) + 1 {
            game.tick();
        }
        assert!(game.played == 1 && !game.overtime && game.score == [0, 0]);
    }

    #[test]
    fn rockets_hit_teammates_too() {
        let mut game = Ballslop::new();
        game.set_bots(0);
        let (a, b, c) = (game.connect("a"), game.connect("b"), game.connect("c"));
        game.phase = Phase::Play;
        assert_eq!(game.players[&a].net.team, game.players[&c].net.team);
        game.ball = Vec3::new(20.0, BALL_RADIUS, 30.0);
        // c, a teammate, stands between a and b; a fires along +x, a little down, at b's legs and hits c's.
        let start = Vec3::new(-14.0, 0.0, 6.0);
        for (id, x) in [(a, 0.0), (c, 5.0), (b, 10.0)] {
            place(&mut game, id, start + Vec3::X * x);
        }
        game.players.get_mut(&c).unwrap().net.hp = 20;
        let fire = Input { fire: true, yaw: -std::f32::consts::FRAC_PI_2, pitch: -0.12, ..Default::default() };
        game.input(a, 1, 0.0, &encode(&fire));
        let mut kills = vec![];
        for _ in 0..ticks(0.5) {
            kills.extend(game.tick().events.iter().filter_map(|e| match decode(&e.data) {
                Some(Event::Kill { killer, victim }) => Some((killer, victim)),
                _ => None,
            }));
        }
        assert_eq!(kills, [(a, c)]);
        assert_eq!((game.players[&a].net.kills, game.players[&c].net.deaths), (-1, 1), "a teamkill costs a kill");
        assert_eq!((game.players[&a].net.hp, game.players[&b].net.hp), (MAX_HP, MAX_HP), "nobody else hurt");
    }

    #[test]
    fn self_damage_is_half_and_everyone_is_thrown() {
        let (mut game, a, b) = duel();
        let team_a = game.players[&a].net.team;
        let at = Vec3::new(-14.0, 0.05, 6.0);
        place(&mut game, a, at.with_y(0.0) + Vec3::X * 1.0);
        place(&mut game, b, at.with_y(0.0) - Vec3::X * 1.0);
        game.explode(at, a, team_a);
        let (pa, pb) = (&game.players[&a].net, &game.players[&b].net);
        let (lost_a, lost_b) = (MAX_HP - pa.hp, MAX_HP - pb.hp);
        assert!((lost_a * 2 - lost_b).abs() <= 1 && (25..40).contains(&lost_a), "self {lost_a}, enemy {lost_b}");
        assert!(pa.state.vel.x > 3.0 && pa.state.vel.y > 3.0 && pb.state.vel.x < -3.0, "{} {}", pa.state.vel, pb.state.vel);
        assert_eq!(game.touched[team_a as usize], None, "the ball is far away");
    }

    #[test]
    fn pickups_are_only_taken_when_useful_and_come_back() {
        let (mut game, a, _) = duel();
        let (i, &(at, _)) = MAP.pickups.iter().enumerate().find(|(_, (_, item))| *item == Item::Health).unwrap();
        place(&mut game, a, at);
        game.tick();
        assert_eq!(game.pickups[i], 0, "full health leaves it");
        game.players.get_mut(&a).unwrap().net.hp = 60;
        place(&mut game, a, at);
        game.tick();
        assert_eq!(game.players[&a].net.hp, 85);
        assert!(game.pickups[i] > game.tick, "taken");
        game.players.get_mut(&a).unwrap().net.hp = 60;
        for _ in 0..ticks(Item::Health.respawn_secs()) - 10 {
            place(&mut game, a, at);
            game.tick();
        }
        assert_eq!(game.players[&a].net.hp, 60, "not back yet");
        for _ in 0..20 {
            place(&mut game, a, at);
            game.tick();
        }
        assert_eq!(game.players[&a].net.hp, 85, "back");
    }

    /// Two full teams of bots for five minutes, no network: they should score and fight, and nothing leaves the map.
    #[test]
    fn bots_play_a_match() {
        let mut game = Ballslop::new();
        game.connect("observer");
        // Kickoffs bring the observer back, so it's taken off the pitch every tick.
        let observer = *game.players.keys().next().unwrap();
        let (mut rockets, mut kills, mut goals, mut touches, mut own, mut team_kills) = (0, 0, 0, 0, 0, 0);
        for _ in 0..ticks(MATCH_SECS) {
            let o = game.players.get_mut(&observer).unwrap();
            (o.net.alive, o.died_at) = (false, u32::MAX / 2);
            for e in game.tick().events {
                match decode::<Event>(&e.data) {
                    Some(Event::Launch { .. }) => rockets += 1,
                    Some(Event::Kill { killer, victim }) => {
                        let team = |id| game.players.get(&id).map(|p: &Player| p.net.team);
                        kills += 1;
                        team_kills += (killer != victim && team(killer) == team(victim)) as u32;
                    }
                    Some(Event::Goal { own_goal, .. }) => (goals, own) = (goals + 1, own + own_goal as u32),
                    Some(Event::BallHit { .. }) => touches += 1,
                    _ => {}
                }
            }
            for p in game.players.values().filter(|p| p.net.alive) {
                assert!(in_bounds(p.net.state.pos), "{} at {:?}", p.net.name, p.net.state.pos);
            }
            assert!(in_bounds(game.ball) && game.ball.x.abs() < 24.0, "ball at {}", game.ball);
        }
        println!("5 min: {goals} goals ({own} own), score {:?}, {kills} kills ({team_kills} teammates), {rockets} rockets, {touches} ball hits", game.score);
        assert!(goals >= 2, "bots should score: {goals}");
        assert!(kills >= 10, "and fight: {kills}");
    }
}

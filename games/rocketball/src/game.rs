//! Rocketball: two teams of four, a giant ball, rocket launchers, knives and two AWPs to fight over. Rockets blast the
//! ball (and home in on it when fired right at it), AWP rounds and knives nudge it and running into it pushes it; most
//! goals in 5 minutes wins, a tie goes to golden goal. Bots fill both teams while anyone is playing.

use crate::bot::{Bot, Role, Seen, View};
use crate::map::{AWP_SPOTS, GOAL_LINE, MAP, MAPS};
use crate::shared::*;
use glam::Vec3;
use sdk::ability::{Payload, step_projectile};
use sdk::{Frame, GameInfo, History, ServerGame, log};
use std::collections::BTreeMap;

/// Players per team while any human is on, unless the host sets another player count: bots make up the rest.
pub const TEAM_SIZE: usize = 4;
const MATCH_SECS: f32 = 300.0;
const OVERTIME_SECS: f32 = 120.0;
const KICKOFF_SECS: f32 = 3.0;
const GOAL_SECS: f32 = 3.0;
const END_SECS: f32 = 8.0;
/// Matches per turn when the host rotates games.
const ROTATE_AFTER: u32 = 2;
pub const RESPAWN_SECS: f32 = 5.0;
/// How near a pickup's spot (or loot) you have to walk to take it.
const PICKUP_REACH: f32 = 1.2;
/// Seconds dropped weapons and ammo packs lie before they go; an AWP then turns up on its spot again.
const LOOT_SECS: f32 = 20.0;

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

    /// Back at `pos` with full health, no armor, the knife and a freshly loaded launcher, facing the center spot.
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
    /// Locked onto the ball, turning after it.
    homing: bool,
}

/// Something lying on the pitch.
struct Dropped {
    id: u32,
    loot: Loot,
    pos: Vec3,
    vel: Vec3,
    /// Whoever dropped it can't take it straight back before this tick.
    owner: u32,
    mine_until: u32,
    /// Gone at this tick; None for an AWP on its spot.
    expires: Option<u32>,
}

/// What teammates, the shooter included, take of a blast's damage.
const FRIENDLY_DAMAGE: f32 = 0.5;

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

pub struct Rocketball {
    map: String,
    players: Players,
    rockets: Vec<Rocket>,
    loot: Vec<Dropped>,
    /// Players and the ball as they were, for lag-compensated AWP shots, slashes and rocket locks.
    history: History,
    tick: u32,
    next_id: u32,
    next_rocket: u32,
    next_loot: u32,
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
    min_players: Option<usize>,
    /// Matches played to the end of their scoreboard since `set_map`, for rotating games.
    played: u32,
}

impl Default for Rocketball {
    fn default() -> Rocketball {
        let mut game = Rocketball {
            map: MAPS[0].into(),
            players: Players::new(),
            rockets: vec![],
            loot: vec![],
            history: History::new(MAX_REWIND_TICKS as usize + 8),
            tick: 0,
            next_id: 1,
            next_rocket: 0,
            next_loot: 0,
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
            min_players: None,
            played: 0,
        };
        game.new_match();
        game
    }
}

impl Rocketball {
    pub fn new() -> Rocketball {
        Rocketball::default()
    }
}

impl ServerGame for Rocketball {
    fn connect(&mut self, who: &sdk::Profile) -> u32 {
        let name = &who.name;
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

    fn input(&mut self, id: u32, seq: u32, view_tick: f32, input: &[u8]) {
        if let (Some(p), Some(inp)) = (self.players.get_mut(&id), decode::<Input>(input)) {
            p.inputs.push(Input { seq, view_tick, ..inp });
        }
    }

    fn tick(&mut self) -> Frame {
        fill_bots(&mut self.players, &mut self.next_id, self.tick, self.min_players);
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
        self.update_loot();
        self.tick += 1;
        self.update_match();

        let mut entities: Vec<Entity> = self.players.values().map(|p| p.net.entity(p.teleport)).collect();
        entities.push(Entity { id: BALL_ID, pos: self.ball, vel: self.ball_vel, teleport: self.ball_teleport, data: encode(&Thing::Ball), ..Default::default() });
        self.history.record(self.tick, &entities);
        entities.extend(self.rockets.iter().map(|r| Entity {
            id: r.id,
            pos: r.pos,
            vel: r.vel,
            data: encode(&Thing::Rocket(NetRocket { owner: r.owner, team: r.team, homing: r.homing })),
            ..Default::default()
        }));
        entities.extend(self.loot.iter().map(|d| Entity { id: d.id, pos: d.pos, yaw: d.id as f32 * 2.4, data: encode(&Thing::Loot(d.loot)), ..Default::default() }));
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
        GameInfo { game: "Rocketball".into(), map: self.map.clone(), maps, players: self.players.len() as u32 - bots, bots }
    }

    fn set_map(&mut self, map: &str) {
        if !MAPS.contains(&map) {
            return log(&format!("no map called {map}; playing {}", self.map));
        }
        (self.map, self.played) = (map.into(), 0);
        self.new_match();
    }

    fn set_min_players(&mut self, players: u32) {
        log(&format!("bots fill up to {players} players"));
        self.min_players = Some(players as usize);
    }
}

impl Rocketball {
    /// Scores, stats, clock and pickups back to the start, nothing on the ground but the AWPs on their spots, and
    /// kickoff.
    fn new_match(&mut self) {
        (self.score, self.clock, self.overtime) = ([0; 2], ticks(MATCH_SECS), false);
        self.pickups = vec![0; MAP.pickups.len()];
        self.loot.clear();
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

    /// One tick of play: bots decide, then every player's inputs since the last tick are applied: weapons are dropped,
    /// rockets leave, AWP rounds and slashes hit.
    fn run_inputs(&mut self) {
        let seen: Vec<Seen> = self.players.values().map(|p| Seen { id: p.net.id, team: p.net.team, pos: p.net.state.pos, vel: p.net.state.vel, alive: p.net.alive }).collect();
        let roles = roles(&self.players, self.ball);
        let pickups: Vec<bool> = self.pickups.iter().map(|&back| self.tick >= back).collect();
        let loot: Vec<(Vec3, Loot)> = self.loot.iter().map(|d| (d.pos, d.loot)).collect();
        let view = View { ball: self.ball, ball_vel: self.ball_vel, players: &seen, pickups: &pickups, loot: &loot };
        for p in self.players.values_mut() {
            if let Some(bot) = &mut p.bot {
                let role = roles.get(&p.net.id).copied().unwrap_or(Role::Support);
                p.inputs.push(bot.think(&p.net, role, &view, self.tick));
            }
        }
        let (mut fired, mut dropped) = (vec![], vec![]);
        for p in self.players.values_mut() {
            for inp in std::mem::take(&mut p.inputs) {
                if !p.net.alive {
                    continue;
                }
                let s = &mut p.net.state;
                let (held, ammo) = (s.primary, s.ammo);
                let step = step_player(s, &inp);
                // G: tossed a little forward (the step itself emptied the hands).
                if let (Some(gun), None) = (held, s.primary) {
                    dropped.push((Loot::Gun(gun, ammo), eye(s) - Vec3::Y * 0.3, aim_dir(s) * 6.0 + Vec3::Y * 2.5 + s.vel, p.net.id));
                }
                if let Some(dir) = step.fired {
                    fired.push((p.net.id, p.net.team, s.gun, eye(s), dir, inp.view_tick));
                }
            }
        }
        for (loot, pos, vel, owner) in dropped {
            self.put(loot, pos, vel, owner, Some(self.tick + ticks(LOOT_SECS)));
        }
        for (shooter, team, gun, from, dir, view_tick) in fired {
            if gun != Gun::Launcher {
                self.strike(shooter, team, gun, from, dir, view_tick);
                continue;
            }
            let id = ROCKET_IDS | (self.next_rocket % ROCKET_IDS);
            self.next_rocket = self.next_rocket.wrapping_add(1);
            let fuse = self.tick + ticks(ROCKET_FUSE);
            // Locked if the ball was by the crosshair where the shooter saw it.
            let homing = lock_on(from, dir, self.rewind(BALL_ID, self.ball, self.ball_teleport, view_tick).map_or(self.ball, |e| e.pos));
            self.rockets.push(Rocket { id, owner: shooter, team, pos: from, vel: dir * ROCKET_SPEED, fuse, homing });
            self.events.push(Event::Launch { shooter, from, dir });
        }
    }

    /// Where entity `id`, now at `pos`, was at `view_tick` (at most `MAX_REWIND_TICKS` back); None if it has
    /// teleported since.
    fn rewind(&self, id: u32, pos: Vec3, teleport: u8, view_tick: f32) -> Option<Entity> {
        let at = view_tick.clamp(self.tick as f32 - MAX_REWIND_TICKS, self.tick as f32);
        self.history.rewind(&Entity { id, pos, teleport, ..Default::default() }, at)
    }

    /// An AWP round or a knife slash: hits the first player (as the shooter saw them, rewound to `view_tick`) or the
    /// ball in its reach, short of the world. Teammates take `FRIENDLY_DAMAGE` as with rockets, a knife in the back
    /// `BACKSTAB` times its damage; the ball is sent on along the shot.
    // ponytail: the ball isn't rewound, so a fast one is hit where it is now. Rewind it too if snipers complain.
    fn strike(&mut self, shooter: u32, team: Team, gun: Gun, from: Vec3, dir: Vec3, view_tick: f32) {
        let ability = gun.ability();
        let Payload::Damage { amount, .. } = ability.payload else { return };
        let (mut hit, mut victim) = (trace_world(from, dir).0.min(ability.range()), None);
        for p in self.players.values().filter(|p| p.net.alive && p.net.id != shooter) {
            let Some(then) = self.rewind(p.net.id, p.net.state.pos, p.teleport, view_tick) else { continue };
            if let Some(t) = trace_capsule(from, dir, then.pos).filter(|&t| t < hit) {
                (hit, victim) = (t, Some((p.net.id, p.net.team, then.yaw)));
            }
        }
        if let Some(t) = trace_ball(from, dir, self.ball).filter(|&t| t < hit) {
            (hit, victim) = (t, None);
            let (push, kind) = if gun == Gun::Awp { (SHOT_PUSH, HitKind::Shot) } else { (KNIFE_POKE, HitKind::Push) };
            self.ball_vel = (self.ball_vel + dir * push).clamp_length_max(BALL_MAX_SPEED);
            self.touch(shooter, team);
            self.events.push(Event::BallHit { pos: self.ball, strength: push, kind });
        }
        if let Some((id, their_team, yaw)) = victim {
            let facing = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            let behind = gun == Gun::Knife && facing.dot(dir.with_y(0.0).normalize_or_zero()) > BACKSTAB_COS;
            let damage = amount * if their_team == team { FRIENDLY_DAMAGE } else { 1.0 } * if behind { BACKSTAB } else { 1.0 };
            self.hurt(shooter, id, damage);
        }
        self.events.push(match gun {
            Gun::Awp => Event::Shot { shooter, from, to: from + dir * hit },
            _ => Event::Slash { shooter, from },
        });
    }

    /// Puts `loot` on the ground at `pos`, thrown at `vel`, until `expires`.
    fn put(&mut self, loot: Loot, pos: Vec3, vel: Vec3, owner: u32, expires: Option<u32>) {
        let id = LOOT_IDS | (self.next_loot % LOOT_IDS);
        self.next_loot = self.next_loot.wrapping_add(1);
        self.loot.push(Dropped { id, loot, pos, vel, owner, mine_until: self.tick + ticks(0.6), expires });
    }

    /// What the dead leave where they fell: their rockets, loaded and reserve, as an ammo pack. Weapons go with them;
    /// an AWP they held turns up again at the center.
    fn drop_from_the_dead(&mut self, id: u32) {
        let s = &mut self.players.get_mut(&id).unwrap().net.state;
        let (pos, vel, (loaded, reserve)) = (s.pos + Vec3::Y * 0.8, s.vel * 0.5, s.ammo);
        let rockets = if s.primary == Some(Gun::Launcher) { loaded as u16 + reserve } else { 0 };
        if rockets > 0 {
            self.put(Loot::Rockets(rockets), pos, vel + Vec3::Y * 1.5, id, Some(self.tick + ticks(LOOT_SECS)));
        }
    }

    /// Loot falls and settles, goes when its time is up, and is taken by the living who walk over it: guns by those
    /// without a primary, rockets by those with a launcher (as many as fit). Then AWPs gone (lain too long, or with
    /// someone who left) turn up again on a free spot, so there are always two.
    fn update_loot(&mut self) {
        for d in &mut self.loot {
            step_projectile(&mut d.pos, &mut d.vel, GRAVITY, 0.25, trace_world);
        }
        let (tick, players, events) = (self.tick, &mut self.players, &mut self.events);
        self.loot.retain_mut(|d| {
            if d.expires.is_some_and(|t| tick >= t) {
                return false;
            }
            let (pos, owner, mine_until) = (d.pos, d.owner, d.mine_until);
            let near = |p: &&mut Player| {
                let to = pos - p.net.state.pos;
                p.net.alive && to.with_y(0.0).length() < PICKUP_REACH && (-0.5..1.5).contains(&to.y) && (p.net.id != owner || tick >= mine_until)
            };
            for p in players.values_mut().filter(near) {
                let s = &mut p.net.state;
                match &mut d.loot {
                    Loot::Gun(gun, ammo) => {
                        if s.pick_up(*gun, *ammo) {
                            return false;
                        }
                    }
                    Loot::Rockets(n) if s.primary == Some(Gun::Launcher) && s.ammo.1 < MAX_RESERVE => {
                        let add = (*n).min(MAX_RESERVE - s.ammo.1);
                        (s.ammo.1, *n) = (s.ammo.1 + add, *n - add);
                        events.push(Event::Pickup { player: p.net.id, kind: Item::Rockets });
                        if *n == 0 {
                            return false;
                        }
                    }
                    Loot::Rockets(_) => {}
                }
            }
            true
        });
        let is_awp = |d: &Dropped| matches!(d.loot, Loot::Gun(Gun::Awp, _));
        let held = self.players.values().filter(|p| p.net.alive && p.net.state.primary == Some(Gun::Awp)).count();
        for _ in held + self.loot.iter().filter(|d| is_awp(d)).count()..AWP_SPOTS.len() {
            let free = AWP_SPOTS.iter().find(|&&at| !self.loot.iter().any(|d| is_awp(d) && d.pos.distance(at) < 1.0));
            self.put(Loot::Gun(Gun::Awp, AWP.full_ammo().unwrap()), *free.unwrap_or(&AWP_SPOTS[0]), Vec3::ZERO, 0, None);
        }
    }

    /// Flies rockets, those locked on turning after the ball, setting off those that touch the world, anyone but the
    /// shooter, or the ball, or whose fuse has burnt.
    fn update_rockets(&mut self) {
        let mut blasts = vec![];
        let ball = self.ball;
        self.rockets.retain_mut(|r| {
            r.homing = r.homing && home(r.pos, &mut r.vel, ball);
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

    /// Armor takes its share until it runs out; at 0 HP the victim dies, dropping their primary and rockets, and the
    /// attacker gets a kill for an enemy and loses one for a teammate.
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
        self.drop_from_the_dead(victim);
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
                self.pickups[i] = self.tick + ticks(item.respawn_secs(self.players.len()));
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
            let (most, add) = match p.state.primary {
                Some(Gun::Launcher) => (MAX_RESERVE, 8),
                Some(Gun::Awp) => (MAX_ROUNDS, 5),
                _ => return false,
            };
            let reserve = &mut p.state.ammo.1;
            let useful = *reserve < most;
            *reserve = (*reserve + add).min(most).max(*reserve);
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

/// Bots wanted per team: none without a human; else enough to make `TEAM_SIZE` a side, or to make the host's player
/// count with the teams as even as they can be.
fn bot_targets(players: &Players, min_players: Option<usize>) -> [usize; 2] {
    let humans = |t: Team| players.values().filter(|p| p.bot.is_none() && p.net.team == t).count();
    let humans = [humans(Team::Orange), humans(Team::Blue)];
    if humans == [0, 0] {
        return [0, 0];
    }
    let Some(n) = min_players else { return humans.map(|h| TEAM_SIZE.saturating_sub(h)) };
    let mut want = [0, 0];
    for _ in 0..n.saturating_sub(humans[0] + humans[1]) {
        want[(humans[1] + want[1] < humans[0] + want[0]) as usize] += 1;
    }
    want
}

/// Adds or removes bots on each team to meet `bot_targets`.
fn fill_bots(players: &mut Players, next_id: &mut u32, tick: u32, min_players: Option<usize>) {
    let want = bot_targets(players, min_players);
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

    fn place(game: &mut Rocketball, id: u32, pos: Vec3) {
        let s = &mut game.players.get_mut(&id).unwrap().net.state;
        (s.pos, s.vel) = (pos, Vec3::ZERO);
    }

    /// Two humans, one a side, no bots, in play.
    fn duel() -> (Rocketball, u32, u32) {
        let mut game = Rocketball::new();
        game.set_min_players(0);
        let (a, b) = (game.connect(&"a".into()), game.connect(&"b".into()));
        game.phase = Phase::Play;
        (game, a, b)
    }

    /// Gives `id` a gun straight into their hands, fully loaded, with no draw.
    fn arm(game: &mut Rocketball, id: u32, gun: Gun) {
        let s = &mut game.players.get_mut(&id).unwrap().net.state;
        (s.gun, s.primary, s.ammo) = (gun, Some(gun).filter(|&g| g != Gun::Knife).or(s.primary), gun.ability().full_ammo().unwrap_or(s.ammo));
    }

    /// Feeds `id` the same input every tick for `secs`, returning the events.
    fn hold(game: &mut Rocketball, id: u32, inp: Input, secs: f32) -> Vec<Event> {
        let mut events = vec![];
        for _ in 0..ticks(secs).max(1) {
            game.input(id, 1, game.tick as f32, &encode(&inp));
            events.extend(game.tick().events.iter().filter_map(|e| decode::<Event>(&e.data)));
        }
        events
    }

    fn awps(game: &Rocketball) -> Vec<Vec3> {
        game.loot.iter().filter(|d| matches!(d.loot, Loot::Gun(Gun::Awp, _))).map(|d| d.pos).collect()
    }

    #[test]
    fn teams_balance_and_bots_fill_each_side() {
        let mut game = Rocketball::new();
        let a = game.connect(&"a".into());
        game.tick();
        let team = |game: &Rocketball, t: Team| game.players.values().filter(|p| p.net.team == t).count();
        assert_eq!((team(&game, Team::Orange), team(&game, Team::Blue)), (TEAM_SIZE, TEAM_SIZE));
        let b = game.connect(&"b".into());
        assert_ne!(game.players[&a].net.team, game.players[&b].net.team, "humans split");
        game.set_min_players(5);
        game.tick();
        assert_eq!((team(&game, Team::Orange), team(&game, Team::Blue)), (3, 2));
        game.set_min_players(0);
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
        let mut game = Rocketball::new();
        game.set_min_players(0);
        let (a, b, c) = (game.connect(&"a".into()), game.connect(&"b".into()), game.connect(&"c".into()));
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
    fn the_awp_kills_where_the_shooter_saw_them_and_nudges_the_ball() {
        let (mut game, a, b) = duel();
        place(&mut game, a, Vec3::new(0.0, 0.0, 30.0));
        place(&mut game, b, Vec3::new(0.0, 0.0, -10.0));
        game.ball = Vec3::new(20.0, BALL_RADIUS, 0.0);
        arm(&mut game, a, Gun::Awp);
        game.tick();
        let seen = game.tick as f32;
        // b steps aside before the shot arrives, but a fired at where they saw b.
        place(&mut game, b, Vec3::new(3.0, 0.0, -10.0));
        let level = (EYE - 1.0f32).atan2(40.0);
        let shot = Input { fire: true, zoom: true, pitch: -level, ..Default::default() };
        game.input(a, 1, seen, &encode(&shot));
        let events: Vec<Event> = game.tick().events.iter().filter_map(|e| decode(&e.data)).collect();
        assert!(events.contains(&Event::Kill { killer: a, victim: b }), "{events:?}");
        // Now at the ball, from beside it.
        game.tick();
        place(&mut game, a, Vec3::new(10.0, 0.0, 0.0));
        let still = game.ball;
        let at_ball = Input { fire: true, zoom: true, yaw: -std::f32::consts::FRAC_PI_2, pitch: -(EYE - BALL_RADIUS).atan2(10.0), ..Default::default() };
        for (secs, inp) in [(AWP.cycle, Input { fire: false, ..at_ball }), (0.5, at_ball)] {
            for _ in 0..ticks(secs) {
                game.input(a, 2, game.tick as f32, &encode(&inp));
                game.tick();
            }
        }
        assert!(game.ball.x > still.x + 1.0 && game.last_touch == Some(game.players[&a].net.team), "{}", game.ball);
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
        for _ in 0..ticks(Item::Health.respawn_secs(game.players.len())) - 10 {
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

    /// Two AWPs lie at the center; one is only taken after dropping the launcher, and one gone from play turns up on
    /// its spot again.
    #[test]
    fn a_center_awp_is_taken_after_dropping_the_launcher() {
        let (mut game, a, _) = duel();
        game.tick();
        assert_eq!(awps(&game), AWP_SPOTS, "both on their spots");
        for _ in 0..ticks(1.0) {
            place(&mut game, a, AWP_SPOTS[0]);
            game.tick();
        }
        assert_eq!((game.players[&a].net.state.primary, awps(&game).len()), (Some(Gun::Launcher), 2), "not with the launcher");
        place(&mut game, a, AWP_SPOTS[0]);
        hold(&mut game, a, Input { drop: true, ..Default::default() }, 0.0);
        let s = game.players[&a].net.state;
        assert_eq!((s.primary, s.ammo, awps(&game).len()), (Some(Gun::Awp), AWP.full_ammo().unwrap(), 1));
        let launcher = game.loot.iter().find(|d| d.loot == Loot::Gun(Gun::Launcher, ROCKET.full_ammo().unwrap())).map(|d| d.id);
        assert!(launcher.is_some(), "the launcher is on the ground");
        hold(&mut game, a, Input::default(), DRAW_SECS);
        assert_eq!(game.players[&a].net.state.gun, Gun::Awp);
        assert!(game.loot.iter().any(|d| Some(d.id) == launcher), "and stays there: we have a primary");
        // Dead, a takes it along, and it turns up back on its spot.
        place(&mut game, a, Vec3::new(10.0, 0.0, 10.0));
        game.hurt(a, a, 500.0);
        game.tick();
        assert_eq!(awps(&game).len(), 2);
        assert!(AWP_SPOTS.iter().all(|&at| awps(&game).contains(&at)), "{:?}", awps(&game));
    }

    #[test]
    fn the_knife_cuts_harder_from_behind_and_pokes_the_ball() {
        let (mut game, a, b) = duel();
        game.ball = Vec3::new(20.0, BALL_RADIUS, 30.0);
        place(&mut game, a, Vec3::new(-14.0, 0.0, 6.0));
        place(&mut game, b, Vec3::new(-14.0, 0.0, 4.8));
        arm(&mut game, a, Gun::Knife);
        game.players.get_mut(&b).unwrap().net.state.yaw = std::f32::consts::PI;
        game.tick();
        let slash = Input { knife: true, fire: true, pitch: -0.3, ..Default::default() };
        hold(&mut game, a, slash, 0.0);
        assert_eq!(game.players[&b].net.hp, MAX_HP - 35, "face to face");
        hold(&mut game, a, Input { fire: false, ..slash }, KNIFE.cycle);
        game.players.get_mut(&b).unwrap().net.hp = MAX_HP;
        game.players.get_mut(&b).unwrap().net.state.yaw = 0.0;
        game.tick();
        hold(&mut game, a, slash, 0.0);
        assert_eq!(game.players[&b].net.hp, MAX_HP - 70, "in the back");
        // Reach is short.
        place(&mut game, b, Vec3::new(-14.0, 0.0, 3.0));
        hold(&mut game, a, Input { fire: false, ..slash }, KNIFE.cycle);
        hold(&mut game, a, slash, 0.0);
        assert_eq!(game.players[&b].net.hp, MAX_HP - 70, "out of reach");
        // The ball, just out of touching range ahead.
        place(&mut game, b, Vec3::new(20.0, 0.0, -30.0));
        (game.ball, game.ball_vel) = (Vec3::new(-14.0, BALL_RADIUS, 6.0 - 1.7), Vec3::ZERO);
        hold(&mut game, a, Input { fire: false, ..slash }, KNIFE.cycle);
        let events = hold(&mut game, a, slash, 0.0);
        assert!(events.iter().any(|e| matches!(e, Event::BallHit { strength: KNIFE_POKE, kind: HitKind::Push, .. })), "{events:?}");
        assert!(game.ball_vel.z < -KNIFE_POKE * 0.8 && game.last_touch == Some(game.players[&a].net.team), "{}", game.ball_vel);
    }

    #[test]
    fn the_dead_drop_their_rockets_as_an_ammo_pack_others_pick_up() {
        let (mut game, a, b) = duel();
        game.tick();
        let at = Vec3::new(10.0, 0.0, 10.0);
        place(&mut game, a, at);
        game.players.get_mut(&a).unwrap().net.state.ammo = (2, 8);
        game.hurt(b, a, 500.0);
        for _ in 0..ticks(1.0) {
            game.tick();
        }
        let dropped: Vec<(Loot, Vec3)> = game.loot.iter().filter(|d| !matches!(d.loot, Loot::Gun(Gun::Awp, _))).map(|d| (d.loot, d.pos)).collect();
        assert_eq!(dropped.iter().map(|d| d.0).collect::<Vec<_>>(), [Loot::Rockets(10)], "loaded and reserve, no launcher");
        assert!(dropped.iter().all(|d| d.1.with_y(0.0).distance(at) < 1.0 && d.1.y < 0.1), "where a fell: {dropped:?}");
        game.players.get_mut(&b).unwrap().net.state.ammo = (4, 3);
        place(&mut game, b, dropped[0].1.with_y(0.0));
        game.tick();
        assert_eq!(game.players[&b].net.state.ammo, (4, 13));
        assert_eq!(game.loot.iter().filter(|d| !matches!(d.loot, Loot::Gun(Gun::Awp, _))).count(), 0, "all of it fit");
        for _ in 0..ticks(LOOT_SECS) {
            game.tick();
        }
        assert_eq!(game.loot.len(), 2, "only the AWPs are left");
    }

    /// A rocket fired right at a ball rolling across: locked on, it curves into it; flying straight, it misses.
    #[test]
    fn a_locked_rocket_curves_into_a_moving_ball() {
        let fire = |homing: bool| {
            let (mut game, a, b) = duel();
            game.tick();
            place(&mut game, b, Vec3::new(20.0, 0.0, -40.0));
            place(&mut game, a, Vec3::new(0.0, 0.0, 24.0));
            (game.ball, game.ball_vel, game.ball_teleport) = (Vec3::new(-6.0, BALL_RADIUS, 0.0), Vec3::X * 10.0, game.ball_teleport + 1);
            let d = (game.ball - eye(&game.players[&a].net.state)).normalize();
            let aim = Input { fire: true, yaw: f32::atan2(-d.x, -d.z), pitch: f32::atan2(d.y, d.with_y(0.0).length()), ..Default::default() };
            let mut events = hold(&mut game, a, aim, 0.0);
            assert_eq!(game.rockets.len(), 1);
            assert!(game.rockets[0].homing, "locked on");
            game.rockets[0].homing = homing;
            let vel = game.rockets[0].vel;
            for _ in 0..ticks(0.3) {
                events.extend(game.tick().events.iter().filter_map(|e| decode::<Event>(&e.data)));
            }
            let turned = game.rockets.first().is_none_or(|r| r.vel != vel);
            for _ in 0..ticks(1.5) {
                events.extend(game.tick().events.iter().filter_map(|e| decode::<Event>(&e.data)));
            }
            let hit = events.iter().find_map(|e| match e {
                Event::BallHit { strength, kind: HitKind::Blast, .. } => Some(*strength),
                _ => None,
            });
            (hit, turned)
        };
        let (hit, turned) = fire(true);
        assert!(turned && hit.is_some_and(|s| s > 0.8 * BLAST_PUSH), "homing: {hit:?}");
        let (hit, turned) = fire(false);
        assert!(!turned && hit.is_none(), "straight: {hit:?}");
    }

    /// Two full teams of bots for five minutes, no network: they should score and fight, and nothing leaves the map.
    #[test]
    fn bots_play_a_match() {
        let mut game = Rocketball::new();
        game.connect(&"observer".into());
        // Kickoffs bring the observer back, so it's taken off the pitch every tick.
        let observer = *game.players.keys().next().unwrap();
        let (mut rockets, mut shots, mut kills, mut goals, mut touches, mut own, mut team_kills) = (0, 0, 0, 0, 0, 0, 0);
        for _ in 0..ticks(MATCH_SECS) {
            let o = game.players.get_mut(&observer).unwrap();
            (o.net.alive, o.died_at) = (false, u32::MAX / 2);
            for e in game.tick().events {
                match decode::<Event>(&e.data) {
                    Some(Event::Launch { .. }) => rockets += 1,
                    Some(Event::Shot { .. }) => shots += 1,
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
        println!("5 min: {goals} goals ({own} own), score {:?}, {kills} kills ({team_kills} teammates), {rockets} rockets, {shots} AWP shots, {touches} ball hits", game.score);
        assert!(goals >= 2, "bots should score: {goals}");
        assert!(kills >= 10, "and fight: {kills}");
    }
}

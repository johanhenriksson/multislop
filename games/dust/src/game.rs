//! Counterslop, bomb defusal: rounds, economy, bots and lag-compensated hitscan. The host owns the network and calls in through
//! `ServerGame`.

use crate::bot::{Bot, Seen, can_see, in_view};
use crate::map::{MAP, MAPS};
use crate::shared::*;
use crate::weapons::*;
use glam::Vec3;
use sdk::ability::{Delivery, Payload, step_projectile};
use sdk::{Entity, Frame, GameInfo, History, ServerGame, log};
use std::collections::BTreeMap;

type Players = BTreeMap<u32, Player>;

struct Player {
    id: u32,
    state: PlayerState,
    hp: i32,
    armor: u8,
    money: u32,
    kills: u32,
    deaths: u32,
    team: Team,
    /// Holding E as of the last simulated input.
    using: bool,
    /// No radio from this player before this tick.
    radio_quiet_until: u32,
    /// Inputs received since the last tick, applied in order on the next one.
    inputs: Vec<Input>,
    /// Bumped on every respawn; see `sdk::Entity::teleport`.
    teleport: u8,
    /// Flashed: can't see (bots can't shoot) until this tick.
    blind_until: u32,
    /// Server-controlled player filling a team slot.
    bot: Option<Bot>,
    name: String,
    ping: u16,
}

impl Player {
    fn new(id: u32, name: String, team: Team, bot: Option<Bot>) -> Player {
        Player {
            id,
            state: PlayerState::default(),
            hp: MAX_HP,
            armor: 0,
            money: START_MONEY,
            kills: 0,
            deaths: 0,
            team,
            using: false,
            radio_quiet_until: 0,
            inputs: Vec::new(),
            teleport: 0,
            blind_until: 0,
            bot,
            name,
            ping: 0,
        }
    }
}

/// Server-side round state; `Round` is the part clients see.
struct Match {
    round: Round,
    phase_ends: u32,
    bomb_explodes: u32,
    /// Who is planting or defusing, and since which tick.
    progress: Option<(u32, u32)>,
    /// Buying is allowed until this tick.
    buy_until: u32,
    /// Consecutive round losses per team, for the loss bonus.
    loss_streak: [u32; 2],
    /// The site the T bots go for this round.
    t_site: char,
    /// Rounds played to the end of their end pause, for rotating games.
    played: u32,
}

/// Rounds per turn when the host rotates games: both halves.
const ROTATE_AFTER: u32 = 2 * HALF;

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

pub struct Dust {
    map: String,
    m: Match,
    players: Players,
    /// Positions of every player per tick, for lag compensation.
    history: History,
    tick: u32,
    next_id: u32,
    /// Shots, damage and radio this tick.
    shots: Vec<Shot>,
    hits: Vec<Hit>,
    radio: Vec<RadioCall>,
    grenades: Vec<Grenade>,
    /// Weapons lying around, and the round they were dropped in (all go at the next).
    dropped: Vec<Dropped>,
    dropped_round: u32,
    /// Players for bots to fill up to, set by the host; None fills both teams to `TEAM_SIZE`.
    min_players: Option<usize>,
}

impl Dust {
    pub fn new() -> Dust {
        Dust {
            map: MAPS[0].into(),
            m: new_match(),
            players: Players::new(),
            history: History::new(MAX_REWIND_TICKS as usize + 8),
            tick: 0,
            next_id: 1,
            shots: Vec::new(),
            hits: Vec::new(),
            radio: Vec::new(),
            grenades: Vec::new(),
            dropped: Vec::new(),
            dropped_round: 0,
            min_players: None,
        }
    }
}

impl ServerGame for Dust {
    fn connect(&mut self, who: &sdk::Profile) -> u32 {
        let team = smaller_team(&self.players);
        let name = clean_name(&who.name);
        let id = self.next_id;
        self.next_id += 1;
        log(&format!("{name} (player {id}) joined as {}", team.name()));
        // Freeze time: spawn now. Otherwise sit out (spectating) until the next round.
        let freeze = self.m.round.phase == Phase::Freeze;
        let index = self.players.values().filter(|c| c.team == team).count();
        let state = spawn_state(team, index);
        self.players.insert(id, Player { state, hp: if freeze { MAX_HP } else { 0 }, ..Player::new(id, name, team, None) });
        id
    }

    fn disconnect(&mut self, id: u32) {
        let Some(c) = self.players.remove(&id) else { return };
        log(&format!("{} left", c.name));
        if self.m.round.bomb == Bomb::Carried(c.id) {
            self.m.round.bomb = Bomb::Dropped(c.state.pos);
        }
    }

    fn ping(&mut self, id: u32, ms: u16) {
        if let Some(c) = self.players.get_mut(&id) {
            c.ping = ms;
        }
    }

    fn input(&mut self, id: u32, seq: u32, view_tick: f32, input: &[u8]) {
        if let (Some(c), Some(inp)) = (self.players.get_mut(&id), decode::<Input>(input)) {
            c.inputs.push(Input { seq, view_tick, ..inp });
        }
    }

    fn tick(&mut self) -> Frame {
        balance_bots(&mut self.m, &mut self.players, &mut self.next_id, self.min_players);
        let alive: Vec<u32> = self.players.values().filter(|c| c.hp > 0).map(|c| c.id).collect();
        let (mut throws, mut dropped) = (Vec::new(), Vec::new());
        let (m, players, history, tick) = (&self.m, &mut self.players, &self.history, self.tick);
        run_inputs(m, players, history, tick, &mut self.shots, &mut self.hits, &mut self.radio, &mut throws, &mut dropped);
        for mut g in throws {
            (g.id, self.next_id) = (self.next_id, self.next_id + 1);
            self.grenades.push(g);
        }
        let (mut booms, mut flashes) = (Vec::new(), Vec::new());
        update_grenades(&mut self.grenades, &mut self.players, self.tick, &mut self.hits, &mut booms, &mut flashes);
        self.tick += 1;
        update_match(&mut self.m, &mut self.players, self.tick);
        drop_from_the_dead(&mut self.players, &alive, &mut dropped);
        if self.m.round.number != self.dropped_round {
            (self.dropped_round, self.dropped) = (self.m.round.number, Vec::new());
        }
        for mut d in dropped {
            (d.id, self.next_id) = (self.next_id, self.next_id + 1);
            self.dropped.push(d);
        }
        if self.dropped.len() > MAX_DROPPED {
            self.dropped.drain(..self.dropped.len() - MAX_DROPPED);
        }
        update_dropped(&mut self.dropped, &mut self.players, self.tick);
        let mut entities = entities(&self.players);
        self.history.record(self.tick, &entities);
        entities.extend(self.grenades.iter().map(|g| Entity {
            id: g.id,
            pos: g.pos,
            vel: g.vel,
            data: encode(&EntityData::Grenade(g.weapon)),
            ..Default::default()
        }));
        entities.extend(self.dropped.iter().map(|d| Entity {
            id: d.id,
            pos: d.pos,
            yaw: d.id as f32 * 2.4,
            data: encode(&EntityData::Dropped(d.weapon)),
            ..Default::default()
        }));

        let everyone = |e: Event| sdk::Event { to: None, data: encode(&e) };
        let mut events: Vec<sdk::Event> = self.shots.drain(..).map(|s| everyone(Event::Shot(s))).collect();
        events.extend(self.hits.drain(..).map(|h| everyone(Event::Hit(h))));
        events.extend(booms.into_iter().map(|(pos, radius)| everyone(Event::Explosion { pos, radius })));
        for f in flashes {
            events.push(everyone(Event::Flashbang { pos: f.pos }));
            events.extend(f.blinded.into_iter().map(|(id, secs)| sdk::Event { to: Some(vec![id]), data: encode(&Event::Flashed { secs }) }));
        }
        for r in self.radio.drain(..) {
            let Some(team) = self.players.get(&r.from).map(|c| c.team) else { continue };
            let to = self.players.values().filter(|c| c.team == team).map(|c| c.id).collect();
            events.push(sdk::Event { to: Some(to), data: encode(&Event::Radio(r)) });
        }
        let finished = self.m.played >= ROTATE_AFTER;
        Frame { map: self.map.clone(), state: encode(&net_round(&self.m, self.tick)), entities, events, finished }
    }

    fn info(&mut self) -> GameInfo {
        let bots = self.players.values().filter(|c| c.bot.is_some()).count() as u32;
        let maps = MAPS.map(String::from).to_vec();
        GameInfo { game: "Counterslop".into(), map: self.map.clone(), maps, players: self.players.len() as u32 - bots, bots }
    }

    fn set_map(&mut self, map: &str) {
        if !crate::map::load(map) {
            return log(&format!("no map called {map}; playing {}", self.map));
        }
        log(&format!("map {map}"));
        // A new match: round 1 starts next tick with everyone respawned and money reset, bots learning the new map.
        (self.map, self.m) = (map.into(), new_match());
        self.grenades.clear();
        self.dropped.clear();
        for c in self.players.values_mut().filter(|c| c.bot.is_some()) {
            c.bot = Some(Bot::new(c.id));
        }
    }

    fn set_min_players(&mut self, players: u32) {
        log(&format!("bots fill up to {players} players"));
        self.min_players = Some(players as usize);
    }
}

fn entities(players: &Players) -> Vec<Entity> {
    players
        .values()
        .map(|c| {
            let p = NetPlayer {
                id: c.id,
                state: c.state,
                hp: c.hp,
                armor: c.armor,
                money: c.money,
                kills: c.kills,
                deaths: c.deaths,
                team: c.team,
                bot: c.bot.is_some(),
                name: c.name.clone(),
                ping: c.ping,
            };
            p.entity(c.teleport)
        })
        .collect()
}

/// One tick of player simulation: bots decide, then every player's inputs since the last tick are applied.
fn run_inputs(
    game: &Match,
    players: &mut Players,
    history: &History,
    tick: u32,
    shots: &mut Vec<Shot>,
    hits: &mut Vec<Hit>,
    radio: &mut Vec<RadioCall>,
    throws: &mut Vec<Grenade>,
    dropped: &mut Vec<Dropped>,
) {
    let seen: Vec<Seen> =
        players.values().map(|c| Seen { id: c.id, team: c.team, pos: c.state.pos, chest: chest(&c.state), alive: c.hp > 0 }).collect();
    // An enemy any living teammate (bot or human) can see right now, so bots can rotate to help.
    let callout = |team: Team| {
        let mut spotters = players.values().filter(|c| c.team == team && c.hp > 0);
        let enemies = || seen.iter().filter(|p| p.alive && p.team != team);
        spotters.find_map(|c| enemies().find(|p| in_view(&c.state, p.chest) && can_see(eye(&c.state), p.chest)).map(|p| p.pos))
    };
    let callouts = [callout(Team::T), callout(Team::Ct)];
    for c in players.values_mut() {
        if let Some(bot) = &mut c.bot {
            let mut inp = bot.think(c.id, &c.state, c.team, &game.round, &seen, callouts[c.team as usize], game.t_site, tick);
            // Flashed: holds fire and the aim drifts.
            if tick < c.blind_until {
                inp.fire = false;
                inp.yaw += (tick as f32 * 0.7 + c.id as f32).sin() * 0.03;
            }
            if let Some(call) = bot.radio.take() {
                radio.push(RadioCall { from: c.id, call });
            }
            c.inputs.push(inp);
        }
    }

    let ids: Vec<u32> = players.keys().copied().collect();
    for id in ids {
        for inp in std::mem::take(&mut player_mut(players, id).inputs) {
            let c = player_mut(players, id);
            c.using = inp.use_key;
            if c.hp <= 0 {
                continue;
            }
            if inp.buy != 0 {
                try_buy(game, c, inp.buy, tick);
            }
            if let Some(&call) = Radio::MENU.get((inp.radio as usize).wrapping_sub(1)) {
                if tick >= c.radio_quiet_until {
                    radio.push(RadioCall { from: c.id, call });
                    c.radio_quiet_until = tick + ticks(2.0);
                }
            }
            // G: what's in hand goes on the ground, tossed a little forward (the step itself empties the slot).
            if let Some((weapon, ammo)) = inp.drop.then(|| c.state.droppable()).flatten() {
                let vel = aim_dir(&c.state) * 6.0 + Vec3::Y * 2.5 + c.state.vel;
                dropped.push(Dropped { id: 0, weapon, ammo, pos: eye(&c.state) - Vec3::Y * 0.3, vel, owner: c.id, mine_until: tick + ticks(0.6) });
            }
            let Some(Fired { weapon, dir, strength }) = step_player(&mut c.state, &inp) else { continue };
            let (shooter, from) = (c.id, eye(&c.state));
            if let Delivery::Projectile { speed, fuse, .. } = weapon.ability().delivery {
                let vel = dir * speed * strength + c.state.vel;
                throws.push(Grenade { id: 0, owner: shooter, weapon, pos: from, vel, fuse: tick + ticks(fuse) });
                // Heard and seen leaving by everyone (a launch flash and whoosh); the projectile itself is an entity.
                shots.push(Shot { shooter, weapon, from, to: from + dir });
                continue;
            }
            let (to, hit) = fire(players, history, tick, shooter, weapon, (from, dir), inp.view_tick);
            shots.push(Shot { shooter, weapon, from, to });
            hits.extend(hit);
        }
    }
}

/// Printable, at most 16 characters, never empty.
fn clean_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(16).collect();
    if name.trim().is_empty() { "player".into() } else { name.trim().into() }
}

fn bot_name(id: u32) -> String {
    const NAMES: [&str; 16] =
        ["Albert", "Bill", "Cliffe", "Gunner", "Hans", "Ivan", "Joe", "Kurt", "Lester", "Moe", "Ringo", "Sergei", "Vladimir", "Yanni", "Zach", "Wolf"];
    format!("BOT {}", NAMES[id as usize % NAMES.len()])
}

/// Balances humans only; bots then fill whichever side is short.
fn smaller_team(players: &Players) -> Team {
    let humans = players.values().filter(|c| c.bot.is_none());
    let ts = humans.clone().filter(|c| c.team == Team::T).count();
    if ts * 2 <= humans.count() { Team::T } else { Team::Ct }
}

/// Teams always have at least this many players; bots fill the gaps.
const TEAM_SIZE: usize = 4;

/// Adds or removes bots so there are `min_players` players, each bot on whichever side is smaller. By default both
/// teams are filled to TEAM_SIZE, or to the bigger human team if larger. No humans, no bots.
fn balance_bots(m: &mut Match, players: &mut Players, next_id: &mut u32, min_players: Option<usize>) {
    let humans = |team: Team| players.values().filter(|c| c.bot.is_none() && c.team == team).count();
    let (ts, cts) = (humans(Team::T), humans(Team::Ct));
    let default = 2 * ts.max(cts).max(TEAM_SIZE);
    let total = if ts + cts == 0 { 0 } else { min_players.unwrap_or(default).saturating_sub(ts + cts) };
    let mut size = [ts, cts];
    for _ in 0..total {
        size[(size[1] < size[0]) as usize] += 1;
    }
    for (team, human_count) in [(Team::T, ts), (Team::Ct, cts)] {
        let want = size[team as usize] - human_count;
        let mut bots: Vec<u32> = players.values().filter(|c| c.bot.is_some() && c.team == team).map(|c| c.id).collect();
        while bots.len() > want {
            let c = players.remove(&bots.pop().unwrap()).unwrap();
            if m.round.bomb == Bomb::Carried(c.id) {
                m.round.bomb = Bomb::Dropped(c.state.pos);
            }
        }
        for _ in bots.len()..want {
            let id = *next_id;
            *next_id += 1;
            let index = players.values().filter(|c| c.team == team).count();
            let freeze = m.round.phase == Phase::Freeze;
            let state = spawn_state(team, index);
            let hp = if freeze { MAX_HP } else { 0 };
            players.insert(id, Player { state, hp, ..Player::new(id, bot_name(id), team, Some(Bot::new(id))) });
            log(&format!("{} joined {}", bot_name(id), team.name()));
        }
    }
}

fn spawn_state(team: Team, index: usize) -> PlayerState {
    let spawns = MAP.spawns(team);
    // Ts face north up their corridor, where the routes to both sites branch off; CTs face south.
    let yaw = if team == Team::T { 0.0 } else { std::f32::consts::PI };
    let mut s = PlayerState { pos: spawns[index % spawns.len()], yaw, frozen: true, ..Default::default() };
    s.loadout(None, Weapon::default_pistol(team));
    s
}

fn new_match() -> Match {
    let round = Round {
        number: 0,
        phase: Phase::End { winner: Team::Ct },
        time_left: 0.0,
        buy_secs: 0.0,
        score: [0, 0],
        bomb: Bomb::None,
        progress: None,
    };
    Match { round, phase_ends: 0, bomb_explodes: 0, progress: None, buy_until: 0, loss_streak: [0, 0], t_site: 'A', played: 0 }
}

/// Teams trade sides: scores go with them, and everyone starts over like round 1, with the pistol and starting money.
fn halftime(m: &mut Match, players: &mut Players) {
    for c in players.values_mut() {
        c.team = if c.team == Team::T { Team::Ct } else { Team::T };
        // Dead as far as `start_round` is concerned, so nobody keeps their guns or armor.
        (c.hp, c.money) = (0, START_MONEY);
    }
    m.round.score.reverse();
    m.loss_streak = [0, 0];
    log(&format!("halftime after round {}: teams switch sides", m.round.number));
}

fn start_round(m: &mut Match, players: &mut Players, tick: u32) {
    m.round.number += 1;
    m.round.phase = Phase::Freeze;
    m.phase_ends = tick + ticks(FREEZE_SECS);
    m.progress = None;
    let mut by_id: Vec<&mut Player> = players.values_mut().collect();
    by_id.sort_by_key(|c| c.id);
    let mut counts = [0, 0];
    for c in by_id {
        let n = &mut counts[c.team as usize];
        let s = &c.state;
        let kept = (c.hp > 0 && m.round.number > 1).then_some((s.primary, s.secondary, s.grenade, s.ammo[2]));
        c.state = spawn_state(c.team, *n);
        c.teleport = c.teleport.wrapping_add(1);
        match kept {
            // Survivors keep their guns, with ammo topped up.
            Some((primary, secondary, grenade, count)) => {
                c.state.loadout(primary, secondary);
                (c.state.grenade, c.state.ammo[2]) = (grenade, count);
            }
            None => c.armor = 0,
        }
        if m.round.number == 1 {
            c.money = START_MONEY;
        }
        c.hp = MAX_HP;
        *n += 1;
        if c.bot.is_some() {
            bot_buy(c);
        }
    }
    m.buy_until = m.phase_ends + ticks(BUY_SECS);
    m.t_site = if (tick.wrapping_mul(2246822519) >> 13) & 1 == 0 { 'A' } else { 'B' };
    let ts: Vec<u32> = players.values().filter(|c| c.team == Team::T).map(|c| c.id).collect();
    m.round.bomb = match ts.len() {
        0 => Bomb::None,
        n => Bomb::Carried(ts[(tick.wrapping_mul(2654435761) >> 16) as usize % n]),
    };
    log(&format!("round {} starts, T bots go {}", m.round.number, m.t_site));
}

fn end_round(m: &mut Match, players: &mut Players, tick: u32, winner: Team, why: &str) {
    m.round.score[winner as usize] += 1;
    let loser = if winner == Team::T { Team::Ct } else { Team::T };
    m.loss_streak[winner as usize] = 0;
    let bonus = LOSS_BONUS * m.loss_streak[loser as usize].min(3);
    m.loss_streak[loser as usize] += 1;
    for c in players.values_mut() {
        let reward = if c.team == winner { WIN_REWARD } else { LOSS_REWARD + bonus };
        c.money = (c.money + reward).min(MAX_MONEY);
    }
    m.round.phase = Phase::End { winner };
    m.phase_ends = tick + ticks(ROUND_END_SECS);
    m.progress = None;
    let [t, ct] = m.round.score;
    log(&format!("round {}: {} win ({why})  T {t} - {ct} CT", m.round.number, winner.name()));
}

/// Advances the round: freeze/live/end transitions, bomb pickup, plant, defuse, explosion and win conditions.
fn update_match(m: &mut Match, players: &mut Players, tick: u32) {
    if players.is_empty() {
        *m = new_match();
        return;
    }
    match m.round.phase {
        Phase::End { .. } => {
            if tick >= m.phase_ends {
                m.played += (m.round.number > 0) as u32;
                if m.round.number > 0 && m.round.number % HALF == 0 {
                    halftime(m, players);
                }
                start_round(m, players, tick);
            }
            return;
        }
        Phase::Freeze => {
            if tick >= m.phase_ends {
                m.round.phase = Phase::Live;
                m.phase_ends = tick + ticks(ROUND_SECS);
                players.values_mut().for_each(|c| c.state.frozen = false);
            }
            return;
        }
        Phase::Live => {}
    }

    let alive = |c: &&Player| c.hp > 0;
    match m.round.bomb {
        Bomb::Carried(id) => {
            let carrier = players.values().find(|c| c.id == id);
            match carrier {
                Some(c) if c.hp > 0 => {
                    let planting = c.using && c.state.on_ground && MAP.site_at(c.state.pos).is_some();
                    m.progress = planting.then(|| m.progress.filter(|p| p.0 == id).unwrap_or((id, tick)));
                    if m.progress.is_some_and(|(_, since)| tick - since >= ticks(PLANT_SECS)) {
                        let site = MAP.site_at(c.state.pos).unwrap_or('?');
                        log(&format!("round {}: bomb planted at {site}", m.round.number));
                        m.round.bomb = Bomb::Planted(c.state.pos);
                        let planter = player_mut(players, id);
                        planter.money = (planter.money + OBJECTIVE_REWARD).min(MAX_MONEY);
                        m.bomb_explodes = tick + ticks(BOMB_SECS);
                        m.progress = None;
                    }
                }
                Some(c) => m.round.bomb = Bomb::Dropped(c.state.pos),
                None => m.round.bomb = Bomb::None,
            }
        }
        Bomb::Dropped(pos) => {
            let near = |c: &&Player| c.team == Team::T && c.state.pos.distance(pos) < BOMB_REACH;
            if let Some(c) = players.values().filter(alive).find(near) {
                m.round.bomb = Bomb::Carried(c.id);
            }
        }
        Bomb::Planted(pos) => {
            let can_defuse = |c: &&Player| {
                c.team == Team::Ct && c.using && c.state.on_ground && c.state.pos.distance(pos) < BOMB_REACH
            };
            let defusers: Vec<u32> = players.values().filter(alive).filter(can_defuse).map(|c| c.id).collect();
            m.progress = match m.progress {
                Some(p) if defusers.contains(&p.0) => Some(p),
                _ => defusers.first().map(|&id| (id, tick)),
            };
            if let Some((defuser, _)) = m.progress.filter(|(_, since)| tick - since >= ticks(DEFUSE_SECS)) {
                m.round.bomb = Bomb::Defused(pos);
                let defuser = player_mut(players, defuser);
                defuser.money = (defuser.money + OBJECTIVE_REWARD).min(MAX_MONEY);
                return end_round(m, players, tick, Team::Ct, "bomb defused");
            }
            if tick >= m.bomb_explodes {
                m.round.bomb = Bomb::Exploded(pos);
                for c in players.values_mut().filter(|c| c.hp > 0 && c.state.pos.distance(pos) < BOMB_RADIUS) {
                    c.hp = 0;
                    c.deaths += 1;
                }
                return end_round(m, players, tick, Team::T, "bomb exploded");
            }
        }
        Bomb::None | Bomb::Exploded(_) | Bomb::Defused(_) => {}
    }

    let count = |team: Team| {
        let members = players.values().filter(|c| c.team == team);
        (members.clone().count(), members.filter(|c| c.hp > 0).count())
    };
    let ((ts, ts_alive), (cts, cts_alive)) = (count(Team::T), count(Team::Ct));
    let planted = matches!(m.round.bomb, Bomb::Planted(_));
    if cts > 0 && cts_alive == 0 {
        end_round(m, players, tick, Team::T, "eliminated");
    } else if ts > 0 && ts_alive == 0 && !planted {
        end_round(m, players, tick, Team::Ct, "eliminated");
    } else if !planted && tick >= m.phase_ends {
        end_round(m, players, tick, Team::Ct, "time ran out");
    }
}

fn net_round(m: &Match, tick: u32) -> Round {
    let until = if matches!(m.round.bomb, Bomb::Planted(_)) { m.bomb_explodes } else { m.phase_ends };
    let needed = if matches!(m.round.bomb, Bomb::Planted(_)) { DEFUSE_SECS } else { PLANT_SECS };
    let buying = matches!(m.round.phase, Phase::Freeze | Phase::Live);
    Round {
        time_left: until.saturating_sub(tick) as f32 / TICK_HZ as f32,
        buy_secs: if buying { m.buy_until.saturating_sub(tick) as f32 / TICK_HZ as f32 } else { 0.0 },
        progress: m.progress.map(|(id, since)| (id, (tick - since) as f32 / TICK_HZ as f32 / needed)),
        ..m.round
    }
}

fn player_mut(players: &mut Players, id: u32) -> &mut Player {
    players.values_mut().find(|c| c.id == id).unwrap()
}

/// Lag-compensated hitscan (and melee, a short one): rewinds everyone else to where the shooter saw them and applies
/// damage by hit zone, distance and armor. Returns where the shot ended and the damage dealt, if any.
fn fire(
    players: &mut Players,
    history: &History,
    tick: u32,
    shooter: u32,
    weapon: Weapon,
    (o, d): (Vec3, Vec3),
    view_tick: f32,
) -> (Vec3, Option<Hit>) {
    let ability = weapon.ability();
    let view_tick = view_tick.clamp(tick as f32 - MAX_REWIND_TICKS, tick as f32);
    let wall = trace_world(o, d).min(ability.range());
    let mut best: Option<(u32, f32, Vec3, f32)> = None;
    let team = players.values().find(|c| c.id == shooter).map(|c| c.team);
    for c in players.values() {
        // Friendly fire is off: shots pass through teammates.
        if c.id == shooter || c.hp <= 0 || Some(c.team) == team {
            continue;
        }
        let now = Entity { id: c.id, pos: c.state.pos, teleport: c.teleport, ..Default::default() };
        let Some(then) = history.rewind(&now, view_tick) else { continue };
        let tall = height(NetPlayer::from_entity(&then).map_or(c.state.duck, |p| p.state.duck));
        if let Some(t) = trace_capsule(o, d, then.pos, tall) {
            if t < wall && best.is_none_or(|b| t < b.1) {
                best = Some((c.id, t, then.pos, tall));
            }
        }
    }
    let Some((victim, t, feet, tall)) = best else { return (o + d * wall, None) };

    // Zones scale with the hull, so a crouched head is still a head.
    let up = ((o + d * t).y - feet.y) / tall * HEIGHT;
    let (headshot, legs) = (up > HEAD_HEIGHT, up < LEGS_HEIGHT);
    let zone = if headshot { 4.0 } else if legs { 0.75 } else { 1.0 };
    let falloff = match ability.delivery {
        Delivery::Hitscan { falloff, .. } => falloff.powf(t / 12.7),
        _ => 1.0,
    };
    let Payload::Damage { amount, armor_pen } = ability.payload else { return (o + d * t, None) };
    let hit = hurt(players, shooter, victim, weapon, amount * falloff * zone, armor_pen, (headshot, legs));
    (o + d * t, Some(hit))
}

/// Applies `damage` (before armor) to `victim`, crediting `attacker` with the kill.
fn hurt(players: &mut Players, attacker: u32, victim: u32, weapon: Weapon, damage: f32, armor_pen: f32, (headshot, legs): (bool, bool)) -> Hit {
    let v = player_mut(players, victim);
    let mut damage = damage;
    if v.armor > 0 && !legs {
        let through = damage * armor_pen;
        v.armor = v.armor.saturating_sub(((damage - through) * 0.5) as u8);
        damage = through;
    }
    let damage = damage.round().max(1.0) as i32;
    v.hp -= damage;
    let killed = v.hp <= 0;
    if killed {
        v.hp = 0;
        v.deaths += 1;
        if let Some(killer) = players.get_mut(&attacker) {
            killer.kills += 1;
            killer.money = (killer.money + weapon.spec().kill_reward).min(MAX_MONEY);
        }
    }
    Hit { attacker, victim, weapon, damage: damage.min(255) as u8, headshot, killed }
}

/// Weapons on the ground at once; the oldest go first.
const MAX_DROPPED: usize = 48;

/// A weapon on the ground, with the ammo it had.
pub struct Dropped {
    id: u32,
    weapon: Weapon,
    ammo: (u8, u16),
    pos: Vec3,
    vel: Vec3,
    /// Whoever dropped it can't pick it straight back up before this tick.
    owner: u32,
    mine_until: u32,
}

/// Those who died this tick (alive before it) drop their best gun and grenade where they fell, keeping the knife.
fn drop_from_the_dead(players: &mut Players, alive: &[u32], dropped: &mut Vec<Dropped>) {
    for c in players.values_mut().filter(|c| c.hp <= 0 && alive.contains(&c.id)) {
        let s = &mut c.state;
        let gun = s.primary.map(|w| (w, s.ammo[0])).or((s.secondary != Weapon::Knife).then_some((s.secondary, s.ammo[1])));
        let grenade = s.grenade.map(|w| (w, s.ammo[2]));
        for (i, (weapon, ammo)) in gun.into_iter().chain(grenade).enumerate() {
            let vel = Vec3::new(i as f32 - 0.5, 1.5, 0.0) + s.vel * 0.5;
            dropped.push(Dropped { id: 0, weapon, ammo, pos: s.pos + Vec3::Y * 0.8, vel, owner: c.id, mine_until: 0 });
        }
        (s.primary, s.secondary, s.grenade, s.slot) = (None, Weapon::Knife, None, Slot::Knife);
    }
}

/// Lets dropped weapons fall and settle, and hands them to the living who walk over them with that slot empty.
fn update_dropped(dropped: &mut Vec<Dropped>, players: &mut Players, tick: u32) {
    for d in dropped.iter_mut() {
        step_projectile(&mut d.pos, &mut d.vel, 20.3, 0.25, trace_world_normal);
    }
    dropped.retain(|d| {
        let near = |c: &&mut Player| {
            let to = d.pos - c.state.pos;
            c.hp > 0 && to.with_y(0.0).length() < 1.0 && (-0.5..1.5).contains(&to.y) && (c.id != d.owner || tick >= d.mine_until)
        };
        !players.values_mut().filter(near).any(|c| c.state.pick_up(d.weapon, d.ammo))
    });
}

/// A thrown grenade on its way.
struct Grenade {
    id: u32,
    owner: u32,
    weapon: Weapon,
    pos: Vec3,
    vel: Vec3,
    /// Goes off at this tick.
    fuse: u32,
}

/// Moves grenades, and sets off those whose fuse has burnt: damage to enemies in the open within the radius.
/// A flashbang that went off, and who it blinded for how long.
#[derive(Default)]
struct Flash {
    pos: Vec3,
    blinded: Vec<(u32, f32)>,
}

/// `booms` gets (where, radius) of each explosion, `flashes` each flashbang.
fn update_grenades(grenades: &mut Vec<Grenade>, players: &mut Players, tick: u32, hits: &mut Vec<Hit>, booms: &mut Vec<(Vec3, f32)>, flashes: &mut Vec<Flash>) {
    for g in grenades.iter_mut() {
        let Delivery::Projectile { gravity, bounce, contact, .. } = g.weapon.ability().delivery else { continue };
        // Contact-fused: a direct hit on an enemy in the way this tick sets it off right there.
        let (step, team) = (g.vel * DT, players.get(&g.owner).map(|c| c.team));
        let direct = players
            .values()
            .filter(|c| contact && c.hp > 0 && Some(c.team) != team)
            .filter_map(|c| trace_capsule(g.pos, step.normalize_or_zero(), c.state.pos, height(c.state.duck)))
            .filter(|&t| t <= step.length())
            .min_by(f32::total_cmp);
        if let Some(t) = direct {
            g.pos += step.normalize_or_zero() * t;
            g.fuse = tick;
            continue;
        }
        if step_projectile(&mut g.pos, &mut g.vel, gravity, bounce, trace_world_normal) && contact {
            g.fuse = tick;
        }
    }
    let (live, done): (Vec<Grenade>, Vec<Grenade>) = std::mem::take(grenades).into_iter().partition(|g| tick < g.fuse);
    *grenades = live;
    for g in done {
        if let Payload::Flash { radius, secs } = g.weapon.ability().payload {
            flashes.push(flash(players, g.pos, radius, secs, tick));
            continue;
        }
        let Payload::Explode { damage, radius, armor_pen } = g.weapon.ability().payload else { continue };
        booms.push((g.pos, radius));
        let team = players.get(&g.owner).map(|c| c.team);
        let center = g.pos + Vec3::Y * 0.1;
        let targets: Vec<(u32, f32)> = players
            .values()
            .filter(|c| c.hp > 0 && Some(c.team) != team)
            .filter_map(|c| {
                let body = c.state.pos + Vec3::Y * 0.9;
                let dist = center.distance(body);
                let open = trace_world(center, (body - center).normalize_or_zero()) >= dist;
                (dist < radius && open).then_some((c.id, damage * (1.0 - dist / radius)))
            })
            .collect();
        for (victim, dmg) in targets {
            hits.push(hurt(players, g.owner, victim, g.weapon, dmg, armor_pen, (false, false)));
        }
    }
}

/// Blinds everyone (thrower and teammates too) who has the flash in sight: longest looking right at it and up close.
fn flash(players: &mut Players, at: Vec3, radius: f32, secs: f32, tick: u32) -> Flash {
    let mut out = Flash { pos: at, blinded: vec![] };
    for c in players.values_mut().filter(|c| c.hp > 0) {
        let eye = eye(&c.state);
        let (to, dist) = ((at - eye).normalize_or_zero(), at.distance(eye));
        if dist > radius || trace_world(eye, to) < dist - 0.2 {
            continue;
        }
        let facing = aim_dir(&c.state).dot(to);
        let look = if facing > 0.5 { 1.0 } else if facing > -0.2 { 0.5 } else { 0.15 };
        let blind = secs * look * (1.0 - 0.6 * dist / radius);
        if blind > 0.3 {
            c.blind_until = c.blind_until.max(tick + ticks(blind));
            out.blinded.push((c.id, blind));
        }
    }
    out
}

/// Spends money on a buy menu item if it's buy time and the player is near their spawn.
fn try_buy(m: &Match, c: &mut Player, item: u8, tick: u32) {
    let buy_time = m.round.phase == Phase::Freeze || (m.round.phase == Phase::Live && tick < m.buy_until);
    let near_spawn = MAP.spawns(c.team).iter().any(|s| s.distance(c.state.pos) < BUY_RADIUS);
    let Some(&buy) = buy_menu(c.team).get((item as usize).wrapping_sub(1)) else { return };
    if !buy_time || !near_spawn || c.hp <= 0 || c.money < buy.price() {
        return;
    }
    match buy {
        Buy::Armor if c.armor >= 100 => return,
        Buy::Armor => c.armor = 100,
        Buy::Weapon(w) if w.spec().team.is_some_and(|t| t != c.team) => return,
        // One at a time, up to as many as it carries, and one kind of grenade at a time.
        Buy::Weapon(w) if w.spec().slot == Slot::Grenade => {
            let most = w.ability().full_ammo().map_or(1, |a| a.0);
            match c.state.grenade {
                None => (c.state.grenade, c.state.ammo[2]) = (Some(w), (1, 0)),
                Some(g) if g == w && c.state.ammo[2].0 < most => c.state.ammo[2].0 += 1,
                Some(_) => return,
            }
        }
        Buy::Weapon(w) if w.spec().slot == Slot::Primary => {
            let secondary = c.state.secondary;
            let ammo = c.state.ammo[1];
            c.state.loadout(Some(w), secondary);
            c.state.ammo[1] = ammo;
        }
        Buy::Weapon(w) => {
            let (primary, ammo) = (c.state.primary, c.state.ammo[0]);
            c.state.loadout(primary, w);
            c.state.ammo[0] = ammo;
            c.state.slot = Slot::Secondary;
        }
    }
    c.money -= buy.price();
}

/// Bots spend at round start: the best gun they can afford, then armor.
fn bot_buy(c: &mut Player) {
    let menu = buy_menu(c.team);
    // Rifle, then MP5, then Deagle; an AWP now and then when rich.
    let picks = if c.money > 6000 && c.id % 4 == 0 { [3, 2, 1, 0] } else { [2, 1, 0, 0] };
    if c.state.primary.is_none() {
        if let Some(&Buy::Weapon(w)) = picks.iter().map(|&i| &menu[i]).find(|b| b.price() <= c.money) {
            if w.spec().slot == Slot::Primary {
                c.state.loadout(Some(w), c.state.secondary);
            } else {
                c.state.loadout(None, w);
            }
            c.money -= w.spec().price;
        }
    }
    if c.armor == 0 && c.money >= ARMOR_PRICE {
        (c.armor, c.money) = (100, c.money - ARMOR_PRICE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: u32, team: Team) -> Player {
        Player::new(id, format!("test{id}"), team, None)
    }

    /// A match with player 1 on T and 2 on CT, advanced into the live phase. Returns the tick.
    fn live() -> (Match, Players, u32) {
        let mut players = Players::new();
        players.insert(1, client(1, Team::T));
        players.insert(2, client(2, Team::Ct));
        let mut m = new_match();
        let mut tick = 1;
        update_match(&mut m, &mut players, tick);
        assert_eq!((m.round.number, m.round.phase, m.round.bomb), (1, Phase::Freeze, Bomb::Carried(1)));
        assert!(players.values().all(|c| c.state.frozen && c.hp == MAX_HP));
        tick += ticks(FREEZE_SECS);
        update_match(&mut m, &mut players, tick);
        assert_eq!(m.round.phase, Phase::Live);
        assert!(players.values().all(|c| !c.state.frozen));
        (m, players, tick)
    }

    fn player(players: &mut Players, id: u32) -> &mut Player {
        players.values_mut().find(|c| c.id == id).unwrap()
    }

    fn run(m: &mut Match, players: &mut Players, tick: &mut u32, secs: f32) {
        for _ in 0..ticks(secs) {
            *tick += 1;
            update_match(m, players, *tick);
        }
    }

    fn site_a() -> Vec3 {
        MAP.site_cells().find(|p| MAP.site_at(*p) == Some('A')).unwrap()
    }

    fn plant(m: &mut Match, players: &mut Players, tick: &mut u32) {
        let t = player(players, 1);
        (t.state.pos, t.state.on_ground, t.using) = (site_a(), true, true);
        run(m, players, tick, PLANT_SECS - 0.1);
        assert_eq!(m.round.bomb, Bomb::Carried(1), "not before 3 s");
        run(m, players, tick, 0.2);
        assert_eq!(m.round.bomb, Bomb::Planted(site_a()));
    }

    #[test]
    fn plant_then_defuse() {
        let (mut m, mut players, mut tick) = live();
        plant(&mut m, &mut players, &mut tick);
        let ct = player(&mut players, 2);
        (ct.state.pos, ct.state.on_ground, ct.using) = (site_a() + Vec3::X, true, true);
        run(&mut m, &mut players, &mut tick, DEFUSE_SECS + 0.1);
        assert_eq!(m.round.bomb, Bomb::Defused(site_a()));
        assert_eq!((m.round.phase, m.round.score), (Phase::End { winner: Team::Ct }, [0, 1]));
        run(&mut m, &mut players, &mut tick, ROUND_END_SECS + 0.1);
        assert_eq!((m.round.number, m.round.phase), (2, Phase::Freeze));
    }

    #[test]
    fn bomb_explodes_and_kills_nearby() {
        let (mut m, mut players, mut tick) = live();
        plant(&mut m, &mut players, &mut tick);
        player(&mut players, 2).state.pos = site_a() + Vec3::X * 5.0;
        run(&mut m, &mut players, &mut tick, BOMB_SECS + 0.1);
        assert_eq!(m.round.phase, Phase::End { winner: Team::T });
        assert!(players.values().all(|c| c.hp == 0), "both within the blast radius");
    }

    #[test]
    fn elimination_and_bomb_drop() {
        let (mut m, mut players, mut tick) = live();
        player(&mut players, 1).state.pos = Vec3::new(3.0, 0.0, 4.0);
        player(&mut players, 1).hp = 0;
        run(&mut m, &mut players, &mut tick, 0.1);
        assert_eq!(m.round.bomb, Bomb::Dropped(Vec3::new(3.0, 0.0, 4.0)));
        assert_eq!(m.round.phase, Phase::End { winner: Team::Ct }, "no Ts left and no bomb planted");
    }

    #[test]
    fn planted_bomb_outlives_the_terrorists() {
        let (mut m, mut players, mut tick) = live();
        plant(&mut m, &mut players, &mut tick);
        player(&mut players, 1).hp = 0;
        run(&mut m, &mut players, &mut tick, 1.0);
        assert_eq!(m.round.phase, Phase::Live, "CT still has to defuse");
    }

    /// Full teams of bots, no network: they should move, fight, plant and finish rounds on their own.
    #[test]
    fn bots_play_rounds() {
        play_rounds("dust");
    }

    #[test]
    fn bots_play_rounds_on_aztec() {
        play_rounds("aztec");
    }

    fn play_rounds(map: &str) {
        assert!(crate::map::load(map));
        let mut players = Players::new();
        for id in 1..=2 * TEAM_SIZE as u32 {
            let team = if id <= TEAM_SIZE as u32 { Team::T } else { Team::Ct };
            let mut c = client(id, team);
            c.bot = Some(Bot::new(id));
            players.insert(id, c);
        }
        let (mut m, mut history, mut shots, mut hits, mut radio) = (new_match(), History::new(64), vec![], vec![], vec![]);
        let (mut tick, mut plants, mut planted) = (1, 0, false);
        // FPS16_ROUNDS plays a longer match, for tuning bots without the noise of 10 rounds.
        let rounds = std::env::var("FPS16_ROUNDS").ok().and_then(|r| r.parse().ok()).unwrap_or(10);
        while m.round.number <= rounds && tick < ticks(rounds as f32 * 3.0 * 60.0) {
            run_inputs(&m, &mut players, &history, tick, &mut shots, &mut hits, &mut radio, &mut vec![], &mut vec![]);
            tick += 1;
            update_match(&mut m, &mut players, tick);
            history.record(tick, &entities(&players));
            let now_planted = matches!(m.round.bomb, Bomb::Planted(_));
            plants += (now_planted && !planted) as u32;
            planted = now_planted;
        }
        let kills: u32 = players.values().map(|c| c.kills).sum();
        let [t, ct] = m.round.score;
        let accuracy = 100.0 * hits.len() as f32 / shots.len().max(1) as f32;
        println!(
            "{map}: {} rounds in {:.0} min, T {t} - {ct} CT, {kills} kills, {plants} plants, {} shots, {accuracy:.0}% accuracy",
            t + ct,
            tick as f32 / TICK_HZ as f32 / 60.0,
            shots.len()
        );
        assert!(t + ct >= rounds, "rounds should finish");
        assert_eq!(m.played, rounds, "a round counts once its end pause is over, for rotating games");
        assert_eq!(players[&1].team == Team::Ct, rounds / HALF % 2 == 1, "sides switch every {HALF} rounds");
        assert!(kills >= 10, "bots should fight");
        assert!(plants >= 1, "Ts should plant at least once");
    }

    #[test]
    fn bots_fill_both_teams() {
        let mut players = Players::new();
        let (mut m, mut next_id) = (new_match(), 100);
        balance_bots(&mut m, &mut players, &mut next_id, None);
        assert!(players.is_empty(), "no humans, no bots");
        let count = |c: &Players, t: Team| c.values().filter(|c| c.team == t).count();
        players.insert(1, client(1, Team::T));
        balance_bots(&mut m, &mut players, &mut next_id, None);
        assert_eq!((count(&players, Team::T), count(&players, Team::Ct)), (TEAM_SIZE, TEAM_SIZE));
        // More humans on one side than a full team: the other side gets bots up to the same size.
        for i in 2..=(TEAM_SIZE + 2) as u32 {
            players.insert(i, client(i, Team::Ct));
        }
        balance_bots(&mut m, &mut players, &mut next_id, None);
        let big = TEAM_SIZE + 1;
        assert_eq!((count(&players, Team::T), count(&players, Team::Ct)), (big, big), "Ts topped up to the human CTs");
        players.retain(|_, c| c.bot.is_some());
        balance_bots(&mut m, &mut players, &mut next_id, None);
        assert!(players.is_empty(), "bots leave with the last human");

        // A set number of players, bots evening out the teams.
        players.insert(1, client(1, Team::T));
        players.insert(2, client(2, Team::T));
        balance_bots(&mut m, &mut players, &mut next_id, Some(5));
        assert_eq!((count(&players, Team::T), count(&players, Team::Ct)), (3, 2));
        balance_bots(&mut m, &mut players, &mut next_id, Some(0));
        assert_eq!(players.len(), 2, "no bots");
    }

    /// The big open yard in the south-east, with a long clear line east.
    fn yard() -> Vec3 {
        MAP.ground(38, 44)
    }

    /// Shoots east across the open yard at player 2, `dist` away, at a given height.
    fn shoot_at(players: &mut Players, weapon: Weapon, dist: f32, height: f32) -> Option<Hit> {
        let start = yard();
        player_mut(players, 2).state.pos = start + Vec3::X * dist;
        fire(players, &History::new(0), 100, 1, weapon, (start + Vec3::Y * height, Vec3::X), 100.0).1
    }

    #[test]
    fn damage_by_zone_armor_and_range() {
        let mut players = Players::new();
        players.insert(1, client(1, Team::T));
        players.insert(2, client(2, Team::Ct));
        let body = shoot_at(&mut players, Weapon::Ak47, 5.0, 1.2).unwrap();
        assert!((34..=36).contains(&body.damage) && !body.headshot && !body.killed, "{body:?}");
        player_mut(&mut players, 2).hp = MAX_HP;
        let head = shoot_at(&mut players, Weapon::Ak47, 5.0, 1.65).unwrap();
        assert!(head.headshot && head.killed, "AK headshot kills: {head:?}");
        assert_eq!(player_mut(&mut players, 1).money, START_MONEY + 300);

        player_mut(&mut players, 2).hp = MAX_HP;
        player_mut(&mut players, 2).armor = 100;
        let armored = shoot_at(&mut players, Weapon::Ak47, 5.0, 1.2).unwrap();
        assert!(armored.damage < body.damage, "armor absorbs some: {armored:?}");
        let legs = shoot_at(&mut players, Weapon::Ak47, 5.0, 0.5).unwrap();
        assert!(legs.damage < body.damage && legs.damage > armored.damage / 2, "legs: {legs:?}");

        player_mut(&mut players, 2).armor = 0;
        let near = shoot_at(&mut players, Weapon::Glock, 2.0, 1.2).unwrap().damage;
        player_mut(&mut players, 2).hp = MAX_HP;
        let far = shoot_at(&mut players, Weapon::Glock, 24.0, 1.2).unwrap().damage;
        assert!(far < near, "pistol falls off with range: {near} -> {far}");
        player_mut(&mut players, 2).hp = MAX_HP;
        assert!(shoot_at(&mut players, Weapon::Knife, 3.0, 1.2).is_none(), "knife reach is short");
    }

    /// Player 1 fires a rocket east at player 2 across the yard: it goes off on contact, long before its fuse. One
    /// fired north into a wall goes off at the wall.
    #[test]
    fn rockets_explode_on_contact() {
        let fire_rocket = |yaw: f32, target: Option<f32>| {
            let (m, mut players, mut tick) = live();
            let start = yard();
            let t = player_mut(&mut players, 1);
            t.state.pos = start;
            t.state.loadout(Some(Weapon::Rocket), Weapon::Glock);
            player_mut(&mut players, 2).state.pos = start + Vec3::X * target.unwrap_or(40.0);
            let aim = Input { yaw, ..Default::default() };
            let script = std::iter::repeat_n(aim, 2).chain(std::iter::once(Input { fire: true, ..aim })).chain(std::iter::repeat_n(aim, 128));
            let (history, mut rockets, mut hits, mut booms) = (History::new(0), vec![], vec![], vec![]);
            let mut boom_tick = None;
            for inp in script {
                player_mut(&mut players, 1).inputs.push(inp);
                run_inputs(&m, &mut players, &history, tick, &mut vec![], &mut hits, &mut vec![], &mut rockets, &mut vec![]);
                update_grenades(&mut rockets, &mut players, tick, &mut hits, &mut booms, &mut vec![]);
                if !booms.is_empty() && boom_tick.is_none() {
                    boom_tick = Some(tick);
                }
                tick += 1;
            }
            (hits, booms, boom_tick.map(|b| b - (tick - 131)))
        };
        let east = -std::f32::consts::FRAC_PI_2;
        let (hits, booms, after) = fire_rocket(east, Some(10.0));
        assert_eq!(booms.len(), 1);
        assert!(after.unwrap() < ticks(0.5), "went off on contact, {after:?} ticks after firing");
        assert!(hits.iter().any(|h| h.victim == 2 && h.weapon == Weapon::Rocket && h.killed), "a direct hit kills: {hits:?}");
        let (_, booms, after) = fire_rocket(0.0, None);
        assert_eq!(booms.len(), 1, "into the wall to the north");
        assert!(after.unwrap() < ticks(1.0));
    }

    /// G drops the gun in hand (never the knife); someone with that slot empty walks over it and has it, ammo and
    /// all. The dropper can't take it straight back.
    #[test]
    fn drop_and_pick_up() {
        let (m, mut players, mut tick) = live();
        let start = yard();
        let t = player_mut(&mut players, 1);
        t.state.pos = start;
        t.state.loadout(Some(Weapon::Ak47), Weapon::Glock);
        t.state.ammo[0] = (17, 60);
        let mut dropped = vec![];
        // Drop the AK, then the pistol it falls back to, then keep pressing G with just the knife left.
        let drop = Input { drop: true, ..Default::default() };
        for inp in [drop, Input::default()].into_iter().chain(std::iter::repeat_n(drop, 70)) {
            player_mut(&mut players, 1).inputs.push(inp);
            run_inputs(&m, &mut players, &History::new(0), tick, &mut vec![], &mut vec![], &mut vec![], &mut vec![], &mut dropped);
            tick += 1;
        }
        let weapons: Vec<(Weapon, (u8, u16))> = dropped.iter().map(|d| (d.weapon, d.ammo)).collect();
        assert_eq!(weapons, [(Weapon::Ak47, (17, 60)), (Weapon::Glock, (20, 120))], "the AK, then the pistol, never the knife");
        let t = &player_mut(&mut players, 1).state;
        assert_eq!((t.primary, t.has(Slot::Secondary), t.slot), (None, false, Slot::Knife));

        // Let them land with player 1 gone off east, then player 2 (no primary) walks over the AK.
        player_mut(&mut players, 1).state.pos = start + Vec3::X * 15.0;
        for _ in 0..ticks(1.0) {
            update_dropped(&mut dropped, &mut players, tick);
            tick += 1;
        }
        let ak = dropped.iter().find(|d| d.weapon == Weapon::Ak47).unwrap().pos;
        let ct = player_mut(&mut players, 2);
        (ct.state.pos, ct.state.primary) = (ak.with_y(MAP.floor_at(ak)), None);
        update_dropped(&mut dropped, &mut players, tick);
        let ct = &player_mut(&mut players, 2).state;
        assert_eq!((ct.primary, ct.ammo[0], ct.slot), (Some(Weapon::Ak47), (17, 60), Slot::Primary));
        assert_eq!(dropped.len(), 1, "the pistol is still there");
    }

    #[test]
    fn the_dead_drop_their_guns() {
        let mut players = Players::new();
        players.insert(1, client(1, Team::T));
        let p = player_mut(&mut players, 1);
        p.state.loadout(Some(Weapon::Awp), Weapon::Glock);
        (p.state.grenade, p.state.ammo[2], p.hp) = (Some(Weapon::He), (1, 0), 0);
        let mut dropped = vec![];
        drop_from_the_dead(&mut players, &[1], &mut dropped);
        let weapons: Vec<Weapon> = dropped.iter().map(|d| d.weapon).collect();
        assert_eq!(weapons, [Weapon::Awp, Weapon::He], "best gun and the grenade");
        drop_from_the_dead(&mut players, &[1], &mut dropped);
        assert_eq!(dropped.len(), 2, "nothing left to drop twice");
    }

    /// A flash blinds whoever can see it: longest looking straight at it, much less facing away, not at all behind
    /// a wall. Teammates and the thrower too.
    #[test]
    fn flashbangs_blind_whoever_sees_them() {
        let mut players = Players::new();
        let start = yard();
        let east = -std::f32::consts::FRAC_PI_2;
        for (id, yaw) in [(1, east), (2, std::f32::consts::FRAC_PI_2), (3, east)] {
            let mut c = client(id, Team::T);
            (c.state.pos, c.state.yaw) = (start, yaw);
            players.insert(id, c);
        }
        // Player 3 stands somewhere close but with a wall in the way.
        let at = start + Vec3::new(6.0, 1.5, 0.0);
        let cells = (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z))).filter(|&(x, z)| MAP.walkable(x, z));
        let hidden = cells.map(|(x, z)| MAP.ground(x, z)).find(|g| {
            let eye = *g + Vec3::Y * EYE;
            g.distance(at) < 20.0 && trace_world(eye, (at - eye).normalize()) < eye.distance(at) - 1.0
        });
        player_mut(&mut players, 3).state.pos = hidden.unwrap();
        let f = flash(&mut players, at, 25.0, 4.0, 100);
        let blind = |id: u32| f.blinded.iter().find(|b| b.0 == id).map(|b| b.1);
        let (looking, away) = (blind(1).unwrap(), blind(2).unwrap());
        assert!(looking > 3.0 && away < looking / 3.0, "looking {looking} s, facing away {away} s");
        assert_eq!(blind(3), None, "behind a wall");
        assert!(player_mut(&mut players, 1).blind_until > 100 + ticks(3.0));
    }

    /// A crouched target is shorter: chest-high shots over it miss, its head is still a head, and the shooter's
    /// lag compensation uses how crouched it was back then.
    #[test]
    fn crouched_targets_are_shorter() {
        let mut players = Players::new();
        players.insert(1, client(1, Team::T));
        players.insert(2, client(2, Team::Ct));
        player_mut(&mut players, 2).state.duck = 1.0;
        assert!(shoot_at(&mut players, Weapon::Ak47, 5.0, 1.3).is_none(), "over the head");
        let head = shoot_at(&mut players, Weapon::Ak47, 5.0, 0.9).unwrap();
        assert!(head.headshot, "{head:?}");

        // Standing now, but crouched at the tick the shooter saw.
        let start = yard();
        let p = player_mut(&mut players, 2);
        (p.hp, p.state.duck, p.state.pos) = (MAX_HP, 0.0, start + Vec3::X * 5.0);
        let mut history = History::new(64);
        let mut crouched = entities(&players);
        crouched.iter_mut().for_each(|e| {
            if let Some(mut p) = NetPlayer::from_entity(e).filter(|p| p.id == 2) {
                p.state.duck = 1.0;
                *e = p.entity(e.teleport);
            }
        });
        history.record(90, &crouched);
        history.record(100, &entities(&players));
        let back_then = fire(&mut players, &history, 100, 1, Weapon::Ak47, (start + Vec3::Y * 1.3, Vec3::X), 90.0).1;
        assert!(back_then.is_none(), "it was crouched at tick 90");
        assert!(fire(&mut players, &history, 100, 1, Weapon::Ak47, (start + Vec3::Y * 1.3, Vec3::X), 100.0).1.is_some());
    }

    #[test]
    fn buying_rules_and_round_money() {
        let (mut m, mut players, tick) = live();
        let t = player_mut(&mut players, 1);
        (t.money, t.state.pos) = (3000, MAP.spawns(Team::T)[0]);
        try_buy(&m, t, 4, tick);
        assert_eq!((t.state.primary, t.money), (None, 3000), "AWP too expensive");
        try_buy(&m, t, 3, tick);
        assert_eq!((t.state.primary, t.money), (Some(Weapon::Ak47), 500), "T rifle is the AK");
        try_buy(&m, t, 5, tick);
        assert_eq!(t.armor, 0, "can't afford armor");
        t.money = 5000;
        t.state.pos = MAP.spawns(Team::Ct)[0];
        try_buy(&m, t, 5, tick);
        assert_eq!(t.armor, 0, "not in the buy zone");
        t.state.pos = MAP.spawns(Team::T)[0];
        try_buy(&m, t, 5, tick + ticks(BUY_SECS) + 1);
        assert_eq!(t.armor, 0, "buy time is over");

        end_round(&mut m, &mut players, tick, Team::Ct, "test");
        assert_eq!(player_mut(&mut players, 1).money, 5000 + LOSS_REWARD);
        assert_eq!(player_mut(&mut players, 2).money, START_MONEY + WIN_REWARD);
        end_round(&mut m, &mut players, tick, Team::Ct, "test");
        assert_eq!(player_mut(&mut players, 1).money, 5000 + 2 * LOSS_REWARD + LOSS_BONUS, "loss streak bonus");
    }

    /// Player 1 winds up an HE grenade and lobs it at player 2, 4 m east in the open yard.
    #[test]
    fn he_grenade_hurts_enemies_nearby() {
        let (m, mut players, mut tick) = live();
        let start = yard();
        let t = player_mut(&mut players, 1);
        (t.state.pos, t.state.grenade, t.state.ammo[2]) = (start, Some(Weapon::He), (1, 0));
        player_mut(&mut players, 2).state.pos = start + Vec3::X * 4.0;
        let aim = Input { yaw: -std::f32::consts::FRAC_PI_2, pitch: -0.3, ..Default::default() };
        let script = std::iter::once(Input { slot: 4, ..aim })
            .chain(std::iter::repeat_n(aim, ticks(DRAW_SECS + 0.1) as usize))
            .chain(std::iter::repeat_n(Input { fire: true, ..aim }, 10))
            .chain(std::iter::repeat_n(aim, ticks(2.5) as usize));
        let (history, mut grenades, mut hits, mut booms) = (History::new(0), vec![], vec![], vec![]);
        for inp in script {
            player_mut(&mut players, 1).inputs.push(inp);
            run_inputs(&m, &mut players, &history, tick, &mut vec![], &mut hits, &mut vec![], &mut grenades, &mut vec![]);
            update_grenades(&mut grenades, &mut players, tick, &mut hits, &mut booms, &mut vec![]);
            tick += 1;
        }
        assert_eq!(booms.len(), 1, "one explosion");
        assert!(hits.iter().any(|h| h.victim == 2 && h.weapon == Weapon::He && h.damage > 20), "{hits:?}");
        let t = player_mut(&mut players, 1);
        assert_eq!((t.state.grenade, t.state.slot), (None, Slot::Secondary), "back to the pistol");
    }

    #[test]
    fn timeout_goes_to_ct() {
        let (mut m, mut players, mut tick) = live();
        run(&mut m, &mut players, &mut tick, ROUND_SECS + 0.1);
        assert_eq!(m.round.phase, Phase::End { winner: Team::Ct });
    }
}

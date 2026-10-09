use crate::bot::{Bot, Seen, can_see};
use crate::map::MAP;
use crate::shared::*;
use bevy::math::Vec3;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Per tick: (tick, [(player id, position, alive)]). Alive is rewound too, so a respawned player can't be hit where they died.
type History = VecDeque<(u32, Vec<(u32, Vec3, bool)>)>;

struct Client {
    id: u32,
    state: PlayerState,
    last_seq: u32,
    hp: i32,
    kills: u32,
    deaths: u32,
    team: Team,
    /// Holding E as of the last simulated input.
    using: bool,
    last_heard: Instant,
    /// Received inputs not yet simulated. Applied one per tick so upstream jitter doesn't make the player teleport.
    queue: VecDeque<Input>,
    last_queued: u32,
    echo: (f64, Instant),
    stats: LinkStats,
    /// Server-controlled player filling a team slot.
    bot: Option<Bot>,
}

/// Per-player connection quality, printed every few seconds so a bad link is easy to spot.
#[derive(Default)]
struct LinkStats {
    packets: u32,
    first_seq: Option<u32>,
    last_packet: Option<Instant>,
    worst_gap: Duration,
    max_queue: usize,
    empty_ticks: u32,
}

/// Server-side round state; `Round` is the part clients see.
struct Match {
    round: Round,
    phase_ends: u32,
    bomb_explodes: u32,
    /// Who is planting or defusing, and since which tick.
    progress: Option<(u32, u32)>,
}

const fn ticks(secs: f32) -> u32 {
    (secs * TICK_HZ as f32) as u32
}

const STATS_EVERY: Duration = Duration::from_secs(5);
/// Above this many queued inputs, simulate two per tick to catch up. Each queued input is a tick of added latency.
const QUEUE_CATCH_UP: usize = INPUT_EVERY as usize + 1;

pub fn run(port: u16) {
    let sock = UdpSocket::bind(("0.0.0.0", port)).expect("bind");
    println!("server build {BUILD} listening on :{port}");
    let mut rejected = std::collections::HashSet::new();

    let mut clients: HashMap<SocketAddr, Client> = HashMap::new();
    // Positions of every player per tick, for lag compensation.
    let mut history: History = VecDeque::new();
    let mut next_id = 1;
    let mut tick: u32 = 0;
    let tick_len = Duration::from_secs_f64(1.0 / TICK_HZ);
    let mut next_tick = Instant::now();
    let mut buf = [0u8; 2048];
    let mut stats_at = Instant::now() + STATS_EVERY;
    let mut game = new_match();

    // Shots since the last snapshot.
    let mut shots = Vec::new();
    loop {
        // Wait on the socket until the tick is due, so packets are handled (and timestamped) the moment they land.
        loop {
            let Some(wait) = next_tick.checked_duration_since(Instant::now()).filter(|w| !w.is_zero()) else { break };
            sock.set_read_timeout(Some(wait)).unwrap();
            let Ok((n, addr)) = sock.recv_from(&mut buf) else { continue };
            let build = postcard::take_from_bytes::<&str>(&buf[..n]).map_or("unknown", |(b, _)| b);
            let packet = (build == BUILD).then(|| decode::<InputPacket>(&buf[..n])).flatten();
            let Some(InputPacket { time, inputs, .. }) = packet else {
                if rejected.insert(addr) {
                    println!("rejected {addr}: client is build {build}, server is build {BUILD}");
                }
                let _ = sock.send_to(&encode(&ServerMsg::WrongBuild { server: BUILD.into() }), addr);
                continue;
            };
            if !clients.contains_key(&addr) {
                let team = smaller_team(&clients);
                println!("player {next_id} joined from {addr} (build {BUILD}) as {}", team.name());
                // Freeze time: spawn now. Otherwise sit out (spectating) until the next round.
                let freeze = game.round.phase == Phase::Freeze;
                let index = clients.values().filter(|c| c.team == team).count();
                clients.insert(addr, Client {
                    id: next_id,
                    state: spawn_state(team, index),
                    last_seq: 0,
                    hp: if freeze { MAX_HP } else { 0 },
                    kills: 0,
                    deaths: 0,
                    team,
                    using: false,
                    last_heard: Instant::now(),
                    queue: VecDeque::new(),
                    last_queued: 0,
                    echo: (0.0, Instant::now()),
                    stats: LinkStats::default(),
                    bot: None,
                });
                next_id += 1;
            }
            let c = clients.get_mut(&addr).unwrap();
            let now = Instant::now();
            c.last_heard = now;
            c.echo = (time, now);
            let st = &mut c.stats;
            st.packets += 1;
            if let Some(last) = st.last_packet.replace(now) {
                st.worst_gap = st.worst_gap.max(now - last);
            }
            if let Some(newest) = inputs.last() {
                st.first_seq.get_or_insert(newest.seq);
            }
            for inp in inputs {
                if inp.seq > c.last_queued {
                    c.last_queued = inp.seq;
                    c.queue.push_back(inp);
                }
            }
        }

        balance_bots(&mut game, &mut clients, &mut next_id);
        run_inputs(&game, &mut clients, &history, tick, &mut shots);

        if Instant::now() >= stats_at {
            stats_at += STATS_EVERY;
            for (addr, c) in clients.iter_mut().filter(|(_, c)| c.bot.is_none()) {
                let st = std::mem::take(&mut c.stats);
                // One packet every INPUT_EVERY client ticks, so the seq span tells how many were sent.
                let sent = st.first_seq.map_or(0, |f| c.last_queued.saturating_sub(f) / INPUT_EVERY + 1);
                let loss = 100.0 * (1.0 - st.packets as f64 / sent.max(1) as f64).max(0.0);
                println!(
                    "player {} ({}): loss {loss:.1}%  worst packet gap {} ms  input buffer max {}  ran dry {} ticks",
                    c.id,
                    addr.ip(),
                    st.worst_gap.as_millis(),
                    st.max_queue,
                    st.empty_ticks
                );
            }
        }

        let gone: Vec<SocketAddr> =
            clients.iter().filter(|(_, c)| c.last_heard.elapsed() > Duration::from_secs(5)).map(|(a, _)| *a).collect();
        for addr in gone {
            let c = clients.remove(&addr).unwrap();
            println!("player {} ({addr}) timed out", c.id);
            if game.round.bomb == Bomb::Carried(c.id) {
                game.round.bomb = Bomb::Dropped(c.state.pos);
            }
        }

        tick += 1;
        update_match(&mut game, &mut clients, tick);

        record(&mut history, &clients, tick);

        if tick % SNAPSHOT_EVERY == 0 {
            let players: Vec<NetPlayer> = clients
                .values()
                .map(|c| NetPlayer { id: c.id, state: c.state, hp: c.hp, kills: c.kills, deaths: c.deaths, team: c.team })
                .collect();
            let round = net_round(&game, tick);
            for (addr, c) in clients.iter().filter(|(_, c)| c.bot.is_none()) {
                let snap = Snapshot {
                    tick,
                    you: c.id,
                    ack: c.last_seq,
                    players: players.clone(),
                    round,
                    shots: shots.clone(),
                    echo: c.echo.0,
                    echo_held: c.echo.1.elapsed().as_secs_f32(),
                };
                let _ = sock.send_to(&encode(&ServerMsg::Snapshot(snap)), addr);
            }
            shots.clear();
        }

        // Late ticks are caught up rather than dropped: a server ticking slower than its clients makes input queues grow.
        next_tick += tick_len;
        let now = Instant::now();
        if now > next_tick + Duration::from_millis(250) {
            next_tick = now;
        }
    }
}

/// One tick of player simulation: bots decide, then every player's next queued input is applied.
fn run_inputs(game: &Match, clients: &mut HashMap<SocketAddr, Client>, history: &History, tick: u32, shots: &mut Vec<Shot>) {
    let seen: Vec<Seen> =
        clients.values().map(|c| Seen { id: c.id, team: c.team, pos: c.state.pos, alive: c.hp > 0 }).collect();
    // An enemy any living teammate (bot or human) can see right now, so bots can rotate to help.
    let callout = |team: Team| {
        let spotters = clients.values().filter(|c| c.team == team && c.hp > 0);
        let spotted = |eye: Vec3| seen.iter().find(|p| p.alive && p.team != team && can_see(eye, p.pos)).map(|p| p.pos);
        spotters.map(|c| eye(&c.state)).find_map(spotted)
    };
    let callouts = [callout(Team::T), callout(Team::Ct)];
    for c in clients.values_mut() {
        if let Some(bot) = &mut c.bot {
            let inp = bot.think(c.id, &c.state, c.team, &game.round, &seen, callouts[c.team as usize], tick);
            (c.last_queued, c.last_heard) = (inp.seq, Instant::now());
            c.queue.push_back(inp);
        }
    }

    let addrs: Vec<SocketAddr> = clients.keys().copied().collect();
    for addr in addrs {
        let c = clients.get_mut(&addr).unwrap();
        c.stats.max_queue = c.stats.max_queue.max(c.queue.len());
        if c.queue.is_empty() {
            c.stats.empty_ticks += 1;
        }
        let n = if c.queue.len() > QUEUE_CATCH_UP { 2 } else { 1 };
        for _ in 0..n {
            let c = clients.get_mut(&addr).unwrap();
            let Some(inp) = c.queue.pop_front() else { break };
            c.last_seq = inp.seq;
            c.using = inp.use_key;
            if c.hp <= 0 {
                continue;
            }
            if step_player(&mut c.state, &inp) {
                let shooter = c.id;
                let shot = (eye(&c.state), aim_dir(&c.state));
                let to = fire(clients, history, tick, shooter, shot, inp.view_tick);
                shots.push(Shot { shooter, from: shot.0, to });
            }
        }
    }
}

/// Remembers where everyone was this tick, for lag compensation.
fn record(history: &mut History, clients: &HashMap<SocketAddr, Client>, tick: u32) {
    history.push_back((tick, clients.values().map(|c| (c.id, c.state.pos, c.hp > 0)).collect()));
    while history.len() > MAX_REWIND_TICKS as usize + 8 {
        history.pop_front();
    }
}

/// Balances humans only; bots then fill whichever side is short.
fn smaller_team(clients: &HashMap<SocketAddr, Client>) -> Team {
    let humans = clients.values().filter(|c| c.bot.is_none());
    let ts = humans.clone().filter(|c| c.team == Team::T).count();
    if ts * 2 <= humans.count() { Team::T } else { Team::Ct }
}

/// Teams always have at least this many players; bots fill the gaps.
const TEAM_SIZE: usize = 3;

/// Adds or removes bots so both teams are TEAM_SIZE, or the bigger human team if larger. No humans, no bots.
fn balance_bots(m: &mut Match, clients: &mut HashMap<SocketAddr, Client>, next_id: &mut u32) {
    let humans = |team: Team| clients.values().filter(|c| c.bot.is_none() && c.team == team).count();
    let (ts, cts) = (humans(Team::T), humans(Team::Ct));
    let size = if ts + cts == 0 { 0 } else { ts.max(cts).max(TEAM_SIZE) };
    for (team, human_count) in [(Team::T, ts), (Team::Ct, cts)] {
        let want = size - human_count;
        let mut bots: Vec<SocketAddr> =
            clients.iter().filter(|(_, c)| c.bot.is_some() && c.team == team).map(|(a, _)| *a).collect();
        while bots.len() > want {
            let c = clients.remove(&bots.pop().unwrap()).unwrap();
            if m.round.bomb == Bomb::Carried(c.id) {
                m.round.bomb = Bomb::Dropped(c.state.pos);
            }
        }
        for _ in bots.len()..want {
            let id = *next_id;
            *next_id += 1;
            // Bots aren't on the network; a reserved fake address just keys them in the client map.
            let addr = SocketAddr::from(([0, 0, 0, 0], (id % 60000) as u16 + 1));
            let index = clients.values().filter(|c| c.team == team).count();
            let freeze = m.round.phase == Phase::Freeze;
            clients.insert(addr, Client {
                id,
                state: spawn_state(team, index),
                last_seq: 0,
                hp: if freeze { MAX_HP } else { 0 },
                kills: 0,
                deaths: 0,
                team,
                using: false,
                last_heard: Instant::now(),
                queue: VecDeque::new(),
                last_queued: 0,
                echo: (0.0, Instant::now()),
                stats: LinkStats::default(),
                bot: Some(Bot::new(id)),
            });
            println!("bot {id} joined {}", team.name());
        }
    }
}

fn spawn_state(team: Team, index: usize) -> PlayerState {
    let spawns = MAP.spawns(team);
    // T spawn is south facing north (-z), CT the other way round.
    let yaw = if team == Team::T { 0.0 } else { std::f32::consts::PI };
    PlayerState { pos: spawns[index % spawns.len()], yaw, frozen: true, ..Default::default() }
}

fn new_match() -> Match {
    let round = Round { number: 0, phase: Phase::End { winner: Team::Ct }, time_left: 0.0, score: [0, 0], bomb: Bomb::None, progress: None };
    Match { round, phase_ends: 0, bomb_explodes: 0, progress: None }
}

fn start_round(m: &mut Match, clients: &mut HashMap<SocketAddr, Client>, tick: u32) {
    m.round.number += 1;
    m.round.phase = Phase::Freeze;
    m.phase_ends = tick + ticks(FREEZE_SECS);
    m.progress = None;
    let mut by_id: Vec<&mut Client> = clients.values_mut().collect();
    by_id.sort_by_key(|c| c.id);
    let mut counts = [0, 0];
    for c in by_id {
        let n = &mut counts[c.team as usize];
        c.state = spawn_state(c.team, *n);
        c.hp = MAX_HP;
        *n += 1;
    }
    let ts: Vec<u32> = clients.values().filter(|c| c.team == Team::T).map(|c| c.id).collect();
    m.round.bomb = match ts.len() {
        0 => Bomb::None,
        n => Bomb::Carried(ts[(tick.wrapping_mul(2654435761) >> 16) as usize % n]),
    };
    println!("round {} starts", m.round.number);
}

fn end_round(m: &mut Match, tick: u32, winner: Team, why: &str) {
    m.round.score[winner as usize] += 1;
    m.round.phase = Phase::End { winner };
    m.phase_ends = tick + ticks(ROUND_END_SECS);
    m.progress = None;
    let [t, ct] = m.round.score;
    println!("round {}: {} win ({why})  T {t} - {ct} CT", m.round.number, winner.name());
}

/// Advances the round: freeze/live/end transitions, bomb pickup, plant, defuse, explosion and win conditions.
fn update_match(m: &mut Match, clients: &mut HashMap<SocketAddr, Client>, tick: u32) {
    if clients.is_empty() {
        *m = new_match();
        return;
    }
    match m.round.phase {
        Phase::End { .. } => {
            if tick >= m.phase_ends {
                start_round(m, clients, tick);
            }
            return;
        }
        Phase::Freeze => {
            if tick >= m.phase_ends {
                m.round.phase = Phase::Live;
                m.phase_ends = tick + ticks(ROUND_SECS);
                clients.values_mut().for_each(|c| c.state.frozen = false);
            }
            return;
        }
        Phase::Live => {}
    }

    let alive = |c: &&Client| c.hp > 0;
    match m.round.bomb {
        Bomb::Carried(id) => {
            let carrier = clients.values().find(|c| c.id == id);
            match carrier {
                Some(c) if c.hp > 0 => {
                    let planting = c.using && c.state.on_ground && MAP.site_at(c.state.pos).is_some();
                    m.progress = planting.then(|| m.progress.filter(|p| p.0 == id).unwrap_or((id, tick)));
                    if m.progress.is_some_and(|(_, since)| tick - since >= ticks(PLANT_SECS)) {
                        let site = MAP.site_at(c.state.pos).unwrap_or('?');
                        println!("round {}: bomb planted at {site}", m.round.number);
                        m.round.bomb = Bomb::Planted(c.state.pos);
                        m.bomb_explodes = tick + ticks(BOMB_SECS);
                        m.progress = None;
                    }
                }
                Some(c) => m.round.bomb = Bomb::Dropped(c.state.pos),
                None => m.round.bomb = Bomb::None,
            }
        }
        Bomb::Dropped(pos) => {
            let near = |c: &&Client| c.team == Team::T && c.state.pos.distance(pos) < BOMB_REACH;
            if let Some(c) = clients.values().filter(alive).find(near) {
                m.round.bomb = Bomb::Carried(c.id);
            }
        }
        Bomb::Planted(pos) => {
            let can_defuse = |c: &&Client| {
                c.team == Team::Ct && c.using && c.state.on_ground && c.state.pos.distance(pos) < BOMB_REACH
            };
            let defusers: Vec<u32> = clients.values().filter(alive).filter(can_defuse).map(|c| c.id).collect();
            m.progress = match m.progress {
                Some(p) if defusers.contains(&p.0) => Some(p),
                _ => defusers.first().map(|&id| (id, tick)),
            };
            if m.progress.is_some_and(|(_, since)| tick - since >= ticks(DEFUSE_SECS)) {
                m.round.bomb = Bomb::Defused(pos);
                return end_round(m, tick, Team::Ct, "bomb defused");
            }
            if tick >= m.bomb_explodes {
                m.round.bomb = Bomb::Exploded(pos);
                for c in clients.values_mut().filter(|c| c.hp > 0 && c.state.pos.distance(pos) < BOMB_RADIUS) {
                    c.hp = 0;
                    c.deaths += 1;
                }
                return end_round(m, tick, Team::T, "bomb exploded");
            }
        }
        Bomb::None | Bomb::Exploded(_) | Bomb::Defused(_) => {}
    }

    let count = |team: Team| {
        let members = clients.values().filter(|c| c.team == team);
        (members.clone().count(), members.filter(|c| c.hp > 0).count())
    };
    let ((ts, ts_alive), (cts, cts_alive)) = (count(Team::T), count(Team::Ct));
    let planted = matches!(m.round.bomb, Bomb::Planted(_));
    if cts > 0 && cts_alive == 0 {
        end_round(m, tick, Team::T, "eliminated");
    } else if ts > 0 && ts_alive == 0 && !planted {
        end_round(m, tick, Team::Ct, "eliminated");
    } else if !planted && tick >= m.phase_ends {
        end_round(m, tick, Team::Ct, "time ran out");
    }
}

fn net_round(m: &Match, tick: u32) -> Round {
    let until = if matches!(m.round.bomb, Bomb::Planted(_)) { m.bomb_explodes } else { m.phase_ends };
    let needed = if matches!(m.round.bomb, Bomb::Planted(_)) { DEFUSE_SECS } else { PLANT_SECS };
    Round {
        time_left: until.saturating_sub(tick) as f32 / TICK_HZ as f32,
        progress: m.progress.map(|(id, since)| (id, (tick - since) as f32 / TICK_HZ as f32 / needed)),
        ..m.round
    }
}

/// Lag-compensated hitscan: rewinds everyone else to where the shooter saw them. Returns where the shot ended.
fn fire(
    clients: &mut HashMap<SocketAddr, Client>,
    history: &History,
    tick: u32,
    shooter: u32,
    (o, d): (Vec3, Vec3),
    view_tick: f32,
) -> Vec3 {
    let view_tick = view_tick.clamp(tick as f32 - MAX_REWIND_TICKS, tick as f32);
    let wall = trace_world(o, d);
    let mut best: Option<(u32, f32)> = None;
    let team = clients.values().find(|c| c.id == shooter).map(|c| c.team);
    for c in clients.values() {
        // Friendly fire is off: shots pass through teammates.
        if c.id == shooter || c.hp <= 0 || Some(c.team) == team {
            continue;
        }
        let (pos, alive_then) = rewind(history, c.id, view_tick).unwrap_or((c.state.pos, true));
        if !alive_then {
            continue;
        }
        if let Some(t) = trace_capsule(o, d, pos) {
            if t < wall && best.is_none_or(|b| t < b.1) {
                best = Some((c.id, t));
            }
        }
    }
    let Some((victim, t)) = best else { return o + d * wall };
    let v = clients.values_mut().find(|c| c.id == victim).unwrap();
    v.hp = 0;
    v.deaths += 1;
    clients.values_mut().find(|c| c.id == shooter).unwrap().kills += 1;
    o + d * t
}

fn rewind(history: &History, id: u32, at: f32) -> Option<(Vec3, bool)> {
    let find = |t: u32| {
        let (_, ps) = history.iter().find(|(ht, _)| *ht == t)?;
        ps.iter().find(|(pid, ..)| *pid == id).map(|(_, p, alive)| (*p, *alive))
    };
    match (find(at.floor() as u32), find(at.ceil() as u32)) {
        (Some(a), Some(b)) => Some((a.0.lerp(b.0, at.fract()), a.1 && b.1)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: u32, team: Team) -> Client {
        Client {
            id,
            state: PlayerState::default(),
            last_seq: 0,
            hp: MAX_HP,
            kills: 0,
            deaths: 0,
            team,
            using: false,
            last_heard: Instant::now(),
            queue: VecDeque::new(),
            last_queued: 0,
            echo: (0.0, Instant::now()),
            stats: LinkStats::default(),
            bot: None,
        }
    }

    /// A match with player 1 on T and 2 on CT, advanced into the live phase. Returns the tick.
    fn live() -> (Match, HashMap<SocketAddr, Client>, u32) {
        let mut clients = HashMap::new();
        clients.insert("1.0.0.1:1".parse().unwrap(), client(1, Team::T));
        clients.insert("1.0.0.2:1".parse().unwrap(), client(2, Team::Ct));
        let mut m = new_match();
        let mut tick = 1;
        update_match(&mut m, &mut clients, tick);
        assert_eq!((m.round.number, m.round.phase, m.round.bomb), (1, Phase::Freeze, Bomb::Carried(1)));
        assert!(clients.values().all(|c| c.state.frozen && c.hp == MAX_HP));
        tick += ticks(FREEZE_SECS);
        update_match(&mut m, &mut clients, tick);
        assert_eq!(m.round.phase, Phase::Live);
        assert!(clients.values().all(|c| !c.state.frozen));
        (m, clients, tick)
    }

    fn player(clients: &mut HashMap<SocketAddr, Client>, id: u32) -> &mut Client {
        clients.values_mut().find(|c| c.id == id).unwrap()
    }

    fn run(m: &mut Match, clients: &mut HashMap<SocketAddr, Client>, tick: &mut u32, secs: f32) {
        for _ in 0..ticks(secs) {
            *tick += 1;
            update_match(m, clients, *tick);
        }
    }

    fn site_a() -> Vec3 {
        MAP.site_cells().find(|p| MAP.site_at(*p) == Some('A')).unwrap()
    }

    fn plant(m: &mut Match, clients: &mut HashMap<SocketAddr, Client>, tick: &mut u32) {
        let t = player(clients, 1);
        (t.state.pos, t.state.on_ground, t.using) = (site_a(), true, true);
        run(m, clients, tick, PLANT_SECS - 0.1);
        assert_eq!(m.round.bomb, Bomb::Carried(1), "not before 3 s");
        run(m, clients, tick, 0.2);
        assert_eq!(m.round.bomb, Bomb::Planted(site_a()));
    }

    #[test]
    fn plant_then_defuse() {
        let (mut m, mut clients, mut tick) = live();
        plant(&mut m, &mut clients, &mut tick);
        let ct = player(&mut clients, 2);
        (ct.state.pos, ct.state.on_ground, ct.using) = (site_a() + Vec3::X, true, true);
        run(&mut m, &mut clients, &mut tick, DEFUSE_SECS + 0.1);
        assert_eq!(m.round.bomb, Bomb::Defused(site_a()));
        assert_eq!((m.round.phase, m.round.score), (Phase::End { winner: Team::Ct }, [0, 1]));
        run(&mut m, &mut clients, &mut tick, ROUND_END_SECS + 0.1);
        assert_eq!((m.round.number, m.round.phase), (2, Phase::Freeze));
    }

    #[test]
    fn bomb_explodes_and_kills_nearby() {
        let (mut m, mut clients, mut tick) = live();
        plant(&mut m, &mut clients, &mut tick);
        player(&mut clients, 2).state.pos = site_a() + Vec3::X * 5.0;
        run(&mut m, &mut clients, &mut tick, BOMB_SECS + 0.1);
        assert_eq!(m.round.phase, Phase::End { winner: Team::T });
        assert!(clients.values().all(|c| c.hp == 0), "both within the blast radius");
    }

    #[test]
    fn elimination_and_bomb_drop() {
        let (mut m, mut clients, mut tick) = live();
        player(&mut clients, 1).state.pos = Vec3::new(3.0, 0.0, 4.0);
        player(&mut clients, 1).hp = 0;
        run(&mut m, &mut clients, &mut tick, 0.1);
        assert_eq!(m.round.bomb, Bomb::Dropped(Vec3::new(3.0, 0.0, 4.0)));
        assert_eq!(m.round.phase, Phase::End { winner: Team::Ct }, "no Ts left and no bomb planted");
    }

    #[test]
    fn planted_bomb_outlives_the_terrorists() {
        let (mut m, mut clients, mut tick) = live();
        plant(&mut m, &mut clients, &mut tick);
        player(&mut clients, 1).hp = 0;
        run(&mut m, &mut clients, &mut tick, 1.0);
        assert_eq!(m.round.phase, Phase::Live, "CT still has to defuse");
    }

    /// 3v3 bots, no network: they should move, fight, plant and finish rounds on their own.
    #[test]
    fn bots_play_rounds() {
        let mut clients = HashMap::new();
        for id in 1..=6 {
            let team = if id <= 3 { Team::T } else { Team::Ct };
            let mut c = client(id, team);
            c.bot = Some(Bot::new(id));
            clients.insert(SocketAddr::from(([0, 0, 0, 0], id as u16)), c);
        }
        let (mut m, mut history, mut shots) = (new_match(), VecDeque::new(), vec![]);
        let (mut tick, mut plants, mut planted) = (1, 0, false);
        while m.round.number <= 10 && tick < ticks(30.0 * 60.0) {
            run_inputs(&m, &mut clients, &history, tick, &mut shots);
            tick += 1;
            update_match(&mut m, &mut clients, tick);
            record(&mut history, &clients, tick);
            let now_planted = matches!(m.round.bomb, Bomb::Planted(_));
            plants += (now_planted && !planted) as u32;
            planted = now_planted;
        }
        let kills: u32 = clients.values().map(|c| c.kills).sum();
        let [t, ct] = m.round.score;
        let accuracy = 100.0 * kills as f32 / shots.len().max(1) as f32;
        println!(
            "{} rounds in {:.0} min, T {t} - {ct} CT, {kills} kills, {plants} plants, {} shots, {accuracy:.0}% accuracy",
            t + ct,
            tick as f32 / TICK_HZ as f32 / 60.0,
            shots.len()
        );
        assert!(t + ct >= 10, "rounds should finish");
        assert!(kills >= 10, "bots should fight");
        assert!(plants >= 1, "Ts should plant at least once");
    }

    #[test]
    fn bots_fill_to_three_a_side() {
        let mut clients = HashMap::new();
        let (mut m, mut next_id) = (new_match(), 10);
        balance_bots(&mut m, &mut clients, &mut next_id);
        assert!(clients.is_empty(), "no humans, no bots");
        let count = |c: &HashMap<SocketAddr, Client>, t: Team| c.values().filter(|c| c.team == t).count();
        clients.insert("1.0.0.1:1".parse().unwrap(), client(1, Team::T));
        balance_bots(&mut m, &mut clients, &mut next_id);
        assert_eq!((count(&clients, Team::T), count(&clients, Team::Ct)), (3, 3));
        for i in 2..=5 {
            clients.insert(SocketAddr::from(([1, 0, 0, i as u8], 1)), client(i, Team::Ct));
        }
        balance_bots(&mut m, &mut clients, &mut next_id);
        assert_eq!((count(&clients, Team::T), count(&clients, Team::Ct)), (4, 4), "4 human CTs: Ts topped up to 4");
        clients.retain(|_, c| c.bot.is_some());
        balance_bots(&mut m, &mut clients, &mut next_id);
        assert!(clients.is_empty(), "bots leave with the last human");
    }

    #[test]
    fn timeout_goes_to_ct() {
        let (mut m, mut clients, mut tick) = live();
        run(&mut m, &mut clients, &mut tick, ROUND_SECS + 0.1);
        assert_eq!(m.round.phase, Phase::End { winner: Team::Ct });
    }
}

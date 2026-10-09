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
    respawn_at: u32,
    last_heard: Instant,
    /// Received inputs not yet simulated. Applied one per tick so upstream jitter doesn't make the player teleport.
    queue: VecDeque<Input>,
    last_queued: u32,
    echo: (f64, Instant),
    stats: LinkStats,
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
                println!("player {next_id} joined from {addr} (build {BUILD})");
                let state = spawn_state(&clients);
                clients.insert(addr, Client {
                    id: next_id,
                    state,
                    last_seq: 0,
                    hp: MAX_HP,
                    kills: 0,
                    deaths: 0,
                    respawn_at: 0,
                    last_heard: Instant::now(),
                    queue: VecDeque::new(),
                    last_queued: 0,
                    echo: (0.0, Instant::now()),
                    stats: LinkStats::default(),
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
                if c.hp <= 0 {
                    continue;
                }
                if step_player(&mut c.state, &inp) {
                    let shooter = c.id;
                    let shot = (eye(&c.state), aim_dir(&c.state));
                    let to = fire(&mut clients, &history, tick, shooter, shot, inp.view_tick);
                    shots.push(Shot { shooter, from: shot.0, to });
                }
            }
        }

        if Instant::now() >= stats_at {
            stats_at += STATS_EVERY;
            for (addr, c) in &mut clients {
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

        clients.retain(|addr, c| {
            let alive = c.last_heard.elapsed() < Duration::from_secs(5);
            if !alive {
                println!("player {} ({addr}) timed out", c.id);
            }
            alive
        });

        tick += 1;
        let ready: Vec<SocketAddr> =
            clients.iter().filter(|(_, c)| c.hp <= 0 && tick >= c.respawn_at).map(|(a, _)| *a).collect();
        for addr in ready {
            let state = spawn_state(&clients);
            let c = clients.get_mut(&addr).unwrap();
            c.hp = MAX_HP;
            c.state = state;
        }

        history.push_back((tick, clients.values().map(|c| (c.id, c.state.pos, c.hp > 0)).collect()));
        while history.len() > MAX_REWIND_TICKS as usize + 8 {
            history.pop_front();
        }

        if tick % SNAPSHOT_EVERY == 0 {
            let players: Vec<NetPlayer> = clients
                .values()
                .map(|c| NetPlayer { id: c.id, state: c.state, hp: c.hp, kills: c.kills, deaths: c.deaths })
                .collect();
            for (addr, c) in &clients {
                let snap = Snapshot {
                    tick,
                    you: c.id,
                    ack: c.last_seq,
                    players: players.clone(),
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

/// Spawn point farthest from every living player.
fn spawn_state(clients: &HashMap<SocketAddr, Client>) -> PlayerState {
    let nearest_enemy = |p: &[f32; 3]| {
        clients.values().filter(|c| c.hp > 0).map(|c| c.state.pos.distance(Vec3::from(*p))).fold(f32::MAX, f32::min)
    };
    let p = *SPAWNS.iter().max_by(|a, b| nearest_enemy(a).total_cmp(&nearest_enemy(b))).unwrap();
    PlayerState { pos: Vec3::from(p), yaw: (p[0].atan2(p[2])), ..Default::default() }
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
    for c in clients.values() {
        if c.id == shooter || c.hp <= 0 {
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
    v.respawn_at = tick + RESPAWN_TICKS;
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

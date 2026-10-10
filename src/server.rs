use crate::net::*;
use sdk::{Event, ServerGame, TICK_HZ, decode, encode};
use sha2::{Digest, Sha256};
use std::hash::BuildHasher;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Hosts games from inside the client: a server on a thread, on the first free LAN port, with `bots` bots (the
/// game's choice when None). The first module plays `map`; more than one means rotating through them. Returns the port.
pub fn host(name: String, modules: Vec<Vec<u8>>, map: String, bots: Option<u32>) -> Result<u16, String> {
    let sock = DISCOVERY_PORTS.clone().find_map(|p| UdpSocket::bind(("0.0.0.0", p)).ok()).ok_or("no free port in 5000-5009")?;
    let port = sock.local_addr().map_err(|e| e.to_string())?.port();
    let (ready, loaded) = std::sync::mpsc::channel();
    std::thread::spawn(move || match load(modules, Some(&map), bots) {
        Ok(games) => {
            let _ = ready.send(Ok(()));
            run(sock, &name, games);
        }
        Err(e) => _ = ready.send(Err(e)),
    });
    loaded.recv().map_err(|e| e.to_string())??;
    Ok(port)
}

/// Loads game modules for `run`, each on its first map (the first one on `map` if given) and with `bots` bots.
pub fn load(modules: Vec<Vec<u8>>, map: Option<&str>, bots: Option<u32>) -> Result<Vec<(Box<dyn ServerGame>, Vec<u8>)>, String> {
    let mut games: Vec<(Box<dyn ServerGame>, Vec<u8>)> = Vec::new();
    for module in modules {
        let mut game = crate::wasm::WasmGame::load(&module).map_err(|e| format!("can't load the game: {e}"))?;
        let first = game.info().maps.first().cloned().unwrap_or_default();
        game.set_map(map.filter(|_| games.is_empty()).unwrap_or(&first));
        if let Some(bots) = bots {
            game.set_bots(bots);
        }
        games.push((Box::new(game), module));
    }
    Ok(games)
}

struct Client {
    /// The id the game gave this player.
    id: u32,
    /// Last input seq handed to the game, acked in snapshots.
    last_seq: u32,
    /// The newest snapshot the client says it has: the next ones go out relative to it.
    has_snapshot: u32,
    last_heard: Instant,
    /// Received inputs not yet simulated. Applied one per tick so upstream jitter doesn't make the player teleport.
    queue: VecDeque<InputMsg>,
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
/// Snapshots kept to diff against (2 s). A client further behind than that gets a whole one.
const SNAPSHOT_HISTORY: usize = 64;
/// Above this many queued inputs, simulate two per tick to catch up. Each queued input is a tick of added latency.
const QUEUE_CATCH_UP: usize = INPUT_EVERY as usize + 1;

/// Runs `games` (each with its wasm, sent to clients that don't have it yet) as `name` on `sock`. With more than one
/// it rotates: when the current game reports `Frame::finished`, everyone leaves it and the next one starts over on
/// its first map, and clients load that game when the server answers their input with it.
pub fn run(sock: UdpSocket, name: &str, mut games: Vec<(Box<dyn ServerGame>, Vec<u8>)>) {
    let port = sock.local_addr().map_or(0, |a| a.port());
    println!("server build {BUILD} listening on :{port}");
    let mut rejected = std::collections::HashSet::new();
    let hashes: Vec<[u8; 32]> = games.iter().map(|(_, module)| Sha256::digest(module).into()).collect();
    // Which game is on, and how many times it has changed (`ServerMsg::Game::game`).
    let (mut turn, mut generation) = (0, 0u8);
    // Each game's tick when it was last on: a game's own clock (which lag compensation rewinds by) only runs while
    // it's on, so snapshots carry that rather than time since the server started.
    let mut paused = vec![0u32; games.len()];
    let rotating = games.len() > 1;
    // Cookies are a keyed hash of the address, so a spoofed hello can't make us send chunks to someone else.
    let cookies = std::hash::RandomState::new();

    let mut clients: HashMap<SocketAddr, Client> = HashMap::new();
    let mut tick: u32 = 0;
    let tick_len = Duration::from_secs_f64(1.0 / TICK_HZ);
    let mut next_tick = Instant::now();
    let mut buf = [0u8; 2048];
    let mut stats_at = Instant::now() + STATS_EVERY;
    // Events since the last snapshot.
    let mut events: Vec<Event> = Vec::new();
    // Recent snapshots (all but the per-client parts), to send new ones relative to whichever one a client has.
    let mut sent: VecDeque<Snapshot> = VecDeque::new();

    loop {
        let (game, module) = &mut games[turn];
        let (hash, size) = (hashes[turn], module.len() as u32);
        // Wait on the socket until the tick is due, so packets are handled (and timestamped) the moment they land.
        loop {
            let Some(wait) = next_tick.checked_duration_since(Instant::now()).filter(|w| !w.is_zero()) else { break };
            sock.set_read_timeout(Some(wait)).unwrap();
            let Ok((n, addr)) = sock.recv_from(&mut buf) else { continue };
            let build = postcard::take_from_bytes::<&str>(&buf[..n]).map_or("unknown", |(b, _)| b);
            let packet = (build == BUILD).then(|| decode::<ClientPacket>(&buf[..n])).flatten();
            let Some(ClientPacket { msg, .. }) = packet else {
                if rejected.insert(addr) {
                    println!("rejected {addr}: client is build {build}, server is build {BUILD}");
                }
                let _ = sock.send_to(&encode(&ServerMsg::WrongBuild { server: BUILD.into() }), addr);
                continue;
            };
            let cookie = cookies.hash_one(addr);
            let InputPacket { time, inputs, who, ping, snapshot, .. } = match msg {
                ClientMsg::Hello { .. } if n >= HELLO_SIZE => {
                    let _ = sock.send_to(&encode(&ServerMsg::Game { hash, size, cookie, game: generation }), addr);
                    continue;
                }
                // Input for the game before: tell them what's on now (never bigger than what they sent).
                ClientMsg::Input(p) if p.game != generation => {
                    let _ = sock.send_to(&encode(&ServerMsg::Game { hash, size, cookie, game: generation }), addr);
                    continue;
                }
                ClientMsg::Info { time, .. } if n >= INFO_QUERY_SIZE => {
                    let mut info = game.info();
                    info.map = info.map.chars().take(NAME_LEN).collect();
                    info.game = info.game.chars().take(NAME_LEN).collect();
                    let name = name.chars().take(NAME_LEN).collect();
                    let _ = sock.send_to(&encode(&ServerMsg::Info { name, game: info, time }), addr);
                    continue;
                }
                ClientMsg::Chunks { cookie: c, chunks } if c == cookie => {
                    for index in chunks.into_iter().take(CHUNKS_PER_REQUEST) {
                        let Some(data) = module.chunks(CHUNK).nth(index as usize) else { continue };
                        let _ = sock.send_to(&encode(&ServerMsg::Chunk { index, data: data.to_vec() }), addr);
                    }
                    continue;
                }
                ClientMsg::Input(p) => p,
                _ => continue,
            };
            let c = clients.entry(addr).or_insert_with(|| {
                let id = game.connect(&who);
                println!("player {id} connected from {addr}");
                Client {
                    id,
                    last_seq: 0,
                    has_snapshot: 0,
                    last_heard: Instant::now(),
                    queue: VecDeque::new(),
                    last_queued: 0,
                    echo: (0.0, Instant::now()),
                    stats: LinkStats::default(),
                }
            });
            let now = Instant::now();
            c.last_heard = now;
            c.echo = (time, now);
            c.has_snapshot = c.has_snapshot.max(snapshot);
            game.ping(c.id, ping);
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

        for c in clients.values_mut() {
            c.stats.max_queue = c.stats.max_queue.max(c.queue.len());
            if c.queue.is_empty() {
                c.stats.empty_ticks += 1;
            }
            let n = if c.queue.len() > QUEUE_CATCH_UP { 2 } else { 1 };
            for inp in c.queue.drain(..n.min(c.queue.len())) {
                c.last_seq = inp.seq;
                game.input(c.id, inp.seq, inp.view_tick, &inp.data);
            }
        }

        if Instant::now() >= stats_at {
            stats_at += STATS_EVERY;
            for (addr, c) in clients.iter_mut() {
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
            let alive = c.last_heard.elapsed() <= Duration::from_secs(5);
            if !alive {
                println!("player {} ({addr}) timed out", c.id);
                game.disconnect(c.id);
            }
            alive
        });

        tick += 1;
        let mut frame = game.tick();
        events.append(&mut frame.events);
        let finished = frame.finished;

        if tick % SNAPSHOT_EVERY == 0 {
            for (addr, c) in &clients {
                let base = sent.iter().find(|s| s.tick == c.has_snapshot);
                let snap = Snapshot {
                    tick,
                    map: frame.map.clone(),
                    you: c.id,
                    ack: c.last_seq,
                    echo: c.echo.0,
                    echo_held: c.echo.1.elapsed().as_secs_f32(),
                    state: frame.state.clone(),
                    entities: frame.entities.clone(),
                    events: events.iter().filter(|e| e.to.as_ref().is_none_or(|to| to.contains(&c.id))).map(|e| e.data.clone()).collect(),
                };
                let _ = sock.send_to(&encode(&ServerMsg::Snapshot(Delta { game: generation, ..diff(base, &snap) })), addr);
            }
            events.clear();
            let world = Snapshot { tick, map: frame.map, you: 0, ack: 0, echo: 0.0, echo_held: 0.0, state: frame.state, entities: frame.entities, events: vec![] };
            sent.push_back(world);
            if sent.len() > SNAPSHOT_HISTORY {
                sent.pop_front();
            }
        }

        if finished && rotating {
            for c in clients.values() {
                game.disconnect(c.id);
            }
            (clients, sent, events) = (HashMap::new(), VecDeque::new(), Vec::new());
            paused[turn] = tick;
            (turn, generation) = ((turn + 1) % games.len(), generation.wrapping_add(1));
            tick = paused[turn];
            let next = &mut games[turn].0;
            let info = next.info();
            let map = info.maps.first().cloned().unwrap_or(info.map);
            next.set_map(&map);
            println!("next game: {} on {map}", info.game);
        }

        // Late ticks are caught up rather than dropped: a server ticking slower than its clients makes input queues grow.
        next_tick += tick_len;
        let now = Instant::now();
        if now > next_tick + Duration::from_millis(250) {
            next_tick = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{Frame, GameInfo};

    /// A game whose turn is over 64 ticks after `set_map`.
    struct Quick(&'static str, u32);

    impl ServerGame for Quick {
        fn connect(&mut self, _: &sdk::Profile) -> u32 {
            1
        }
        fn disconnect(&mut self, _: u32) {}
        fn ping(&mut self, _: u32, _: u16) {}
        fn input(&mut self, _: u32, _: u32, _: f32, _: &[u8]) {}
        fn tick(&mut self) -> Frame {
            self.1 += 1;
            Frame { map: self.0.into(), finished: self.1 > 64, ..Default::default() }
        }
        fn info(&mut self) -> GameInfo {
            GameInfo { game: self.0.into(), maps: vec![self.0.into()], ..Default::default() }
        }
        fn set_map(&mut self, _: &str) {
            self.1 = 0;
        }
        fn set_bots(&mut self, _: u32) {}
    }

    /// A client playing along: told the next game when its input is for the one before, then snapshots of that one.
    #[test]
    fn rotation_brings_clients_to_the_next_game() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        std::thread::spawn(move || run(sock, "test", vec![(Box::new(Quick("a", 0)), vec![1; 10]), (Box::new(Quick("b", 0)), vec![2; 20])]));

        let me = UdpSocket::bind("127.0.0.1:0").unwrap();
        me.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let send = |msg| me.send_to(&encode(&ClientPacket { build: BUILD.into(), msg }), addr).unwrap();
        send(ClientMsg::Hello { pad: vec![0; HELLO_SIZE] });
        let (mut generation, mut seq, mut seen) = (None, 0, Vec::new());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = [0u8; 2048];
        while seen.len() < 2 && Instant::now() < deadline {
            if let Some(game) = generation {
                seq += 1;
                let inputs = vec![InputMsg { seq, view_tick: 0.0, data: vec![] }];
                send(ClientMsg::Input(InputPacket { who: "me".into(), ping: 0, time: 0.0, snapshot: 0, inputs, game }));
            }
            let Ok(n) = me.recv(&mut buf) else { continue };
            match decode::<ServerMsg>(&buf[..n]) {
                Some(ServerMsg::Game { size, game, .. }) if generation != Some(game) => {
                    generation = Some(game);
                    seen.push((game, size));
                }
                Some(ServerMsg::Snapshot(d)) => {
                    assert_eq!(Some(d.game), generation, "only snapshots of the game we're in");
                    if let Some(map) = d.map {
                        assert_eq!(map, ["a", "b"][d.game as usize % 2]);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(seen, vec![(0, 10), (1, 20)], "first game a, then told of b");
    }
}

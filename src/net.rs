//! What the host sends over the wire. Game data rides inside as opaque bytes.

use bevy::math::Vec3;
use sdk::{Entity, GameInfo};
use serde::{Deserialize, Serialize};

/// Bumped by every `tools/release.sh publish`. Server and client must match exactly.
pub const BUILD: &str = env!("FPS16_BUILD");

/// Snapshots go out every this many ticks (32 Hz). Fewer, slightly bigger packets suit Wi-Fi better.
pub const SNAPSHOT_EVERY: u32 = 4;
/// Clients send inputs every this many ticks (64 Hz); each packet carries several so none are lost.
pub const INPUT_EVERY: u32 = 2;

#[derive(Serialize, Deserialize, Debug)]
pub enum ServerMsg {
    /// Keep as the first variant so its encoding never changes between builds.
    WrongBuild { server: String },
    /// A snapshot as the changes from one the client has (see `diff`).
    Snapshot(Delta),
    /// The game this server runs: the sha256 and size of its module. `cookie` must come back with chunk requests, so
    /// only the real sender of a hello can make the server send chunks. `game` counts the server's game changes;
    /// it is also the answer to input for an earlier one, so clients notice the change and load the new game.
    Game { hash: [u8; 32], size: u32, cookie: u64, game: u8 },
    Chunk { index: u32, data: Vec<u8> },
    /// For server browsers: who we are and what's on, with the query's `time` echoed to measure ping.
    Info { name: String, game: GameInfo, time: f64 },
}

/// Ports server browsers ask on: the default and the next few, for several servers on one machine.
pub const DISCOVERY_PORTS: std::ops::RangeInclusive<u16> = 5000..=5009;
/// An info query must be at least this big; names in the answer are cut to `NAME_LEN` so it's never bigger.
pub const INFO_QUERY_SIZE: usize = 160;
pub const NAME_LEN: usize = 32;

/// Module bytes per `ServerMsg::Chunk`, small enough for one unfragmented UDP packet.
pub const CHUNK: usize = 1200;
/// Most chunks sent for one request.
pub const CHUNKS_PER_REQUEST: usize = 32;
/// A hello must be at least this big, so the answer is never larger than the question.
pub const HELLO_SIZE: usize = 64;

/// Every packet a client sends.
#[derive(Serialize, Deserialize, Debug)]
pub struct ClientPacket {
    /// Must stay the first field: the server reads it alone before decoding the rest, so a mismatch is caught even
    /// when the rest of the format has changed.
    pub build: String,
    pub msg: ClientMsg,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientMsg {
    /// Which game do you run? Padded up to `HELLO_SIZE`.
    Hello { pad: Vec<u8> },
    /// What are you? Padded up to `INFO_QUERY_SIZE`. Server browsers broadcast this.
    Info { time: f64, pad: Vec<u8> },
    /// Module chunks wanted, by index.
    Chunks { cookie: u64, chunks: Vec<u32> },
    Input(InputPacket),
}

/// A whole snapshot, as the client's netcode keeps them. On the wire it travels as a `Delta`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: u32,
    /// The map being played; clients rebuild their world when it changes.
    pub map: String,
    /// The receiving player's entity id.
    pub you: u32,
    /// Last input seq the server processed for the receiving client.
    pub ack: u32,
    /// Latest `InputPacket::time` from this client, and how long the server held it before sending this snapshot.
    pub echo: f64,
    pub echo_held: f32,
    pub state: Vec<u8>,
    pub entities: Vec<Entity>,
    /// Events for this client since the last snapshot.
    pub events: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InputMsg {
    pub seq: u32,
    /// The server tick the client was showing remote entities at.
    pub view_tick: f32,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InputPacket {
    /// Player name: the logged-in user on the client's machine.
    pub name: String,
    /// The client's measured round trip, shown on everyone's scoreboard.
    pub ping: u16,
    /// Client clock when sent, echoed back in snapshots to measure the network round trip.
    pub time: f64,
    /// The newest snapshot we have, which the server sends the next ones relative to.
    pub snapshot: u32,
    /// Which of the server's games this is for (`ServerMsg::Game::game`); input for another is answered, not played.
    pub game: u8,
    /// The most recent unacked inputs, oldest first. Redundant on purpose to survive packet loss.
    pub inputs: Vec<InputMsg>,
}

/// A snapshot relative to an earlier one the client has: only entities that changed, only their changed fields, and
/// game data as patches of the bytes that changed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Delta {
    pub tick: u32,
    /// The snapshot this is relative to, or None when it's whole.
    pub base: Option<u32>,
    pub you: u32,
    pub ack: u32,
    pub echo: f64,
    pub echo_held: f32,
    /// Which of the server's games it's from (`ServerMsg::Game::game`).
    pub game: u8,
    /// None: as in the base.
    pub map: Option<String>,
    pub state: Option<Patch>,
    /// Entities that are new or changed.
    pub entities: Vec<EntityDelta>,
    /// Entities in the base that are gone.
    pub removed: Vec<u32>,
    pub events: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct EntityDelta {
    pub id: u32,
    pub pos: Option<Vec3>,
    pub vel: Option<Vec3>,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
    pub teleport: Option<u8>,
    pub data: Option<Patch>,
}

/// New bytes for something: all of them, or (offset, bytes) runs over the old ones when the length is the same.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Patch {
    All(Vec<u8>),
    Runs(Vec<(u16, Vec<u8>)>),
}

impl Patch {
    /// None when nothing changed.
    fn new(old: &[u8], new: &[u8]) -> Option<Patch> {
        if old == new {
            return None;
        }
        if old.len() != new.len() || new.len() > u16::MAX as usize {
            return Some(Patch::All(new.to_vec()));
        }
        let mut runs: Vec<(u16, Vec<u8>)> = Vec::new();
        for (i, (&a, &b)) in old.iter().zip(new).enumerate() {
            if a == b {
                continue;
            }
            // Bridge gaps of a couple of bytes: a run costs about three bytes of its own.
            match runs.last_mut() {
                Some((at, bytes)) if i - (*at as usize + bytes.len()) <= 2 => bytes.extend_from_slice(&new[*at as usize + bytes.len()..=i]),
                _ => runs.push((i as u16, vec![b])),
            }
        }
        let patched: usize = runs.iter().map(|(_, b)| b.len() + 3).sum();
        Some(if patched < new.len() { Patch::Runs(runs) } else { Patch::All(new.to_vec()) })
    }

    fn apply(self, old: &[u8]) -> Option<Vec<u8>> {
        match self {
            Patch::All(bytes) => Some(bytes),
            Patch::Runs(runs) => {
                let mut out = old.to_vec();
                for (at, bytes) in runs {
                    out.get_mut(at as usize..at as usize + bytes.len())?.copy_from_slice(&bytes);
                }
                Some(out)
            }
        }
    }
}

/// `now` relative to `base` (a whole snapshot when there is none).
pub fn diff(base: Option<&Snapshot>, now: &Snapshot) -> Delta {
    let empty = Snapshot { tick: 0, map: String::new(), you: 0, ack: 0, echo: 0.0, echo_held: 0.0, state: vec![], entities: vec![], events: vec![] };
    let b = base.unwrap_or(&empty);
    let changed = |old: f32, new: f32| (old != new).then_some(new);
    let entities = now
        .entities
        .iter()
        .filter_map(|e| {
            let Some(o) = b.entities.iter().find(|o| o.id == e.id) else {
                let d = EntityDelta { id: e.id, pos: Some(e.pos), vel: Some(e.vel), yaw: Some(e.yaw), pitch: Some(e.pitch), teleport: Some(e.teleport), data: Some(Patch::All(e.data.clone())) };
                return Some(d);
            };
            let d = EntityDelta {
                id: e.id,
                pos: (o.pos != e.pos).then_some(e.pos),
                vel: (o.vel != e.vel).then_some(e.vel),
                yaw: changed(o.yaw, e.yaw),
                pitch: changed(o.pitch, e.pitch),
                teleport: (o.teleport != e.teleport).then_some(e.teleport),
                data: Patch::new(&o.data, &e.data),
            };
            (d != EntityDelta { id: e.id, ..Default::default() }).then_some(d)
        })
        .collect();
    let removed = b.entities.iter().filter(|o| !now.entities.iter().any(|e| e.id == o.id)).map(|o| o.id).collect();
    Delta {
        tick: now.tick,
        base: base.map(|b| b.tick),
        you: now.you,
        ack: now.ack,
        echo: now.echo,
        echo_held: now.echo_held,
        game: 0,
        map: (b.map != now.map || base.is_none()).then(|| now.map.clone()),
        state: Patch::new(&b.state, &now.state).or_else(|| base.is_none().then(|| Patch::All(now.state.clone()))),
        entities,
        removed,
        events: now.events.clone(),
    }
}

/// The whole snapshot back from a delta and its base. None if it needs a base we don't have, or doesn't fit it.
pub fn apply(base: Option<&Snapshot>, d: Delta) -> Option<Snapshot> {
    let base = match d.base {
        Some(tick) => Some(base.filter(|b| b.tick == tick)?),
        None => None,
    };
    let (map, state, mut entities) = match base {
        Some(b) => (b.map.clone(), b.state.clone(), b.entities.clone()),
        None => (String::new(), vec![], vec![]),
    };
    entities.retain(|e| !d.removed.contains(&e.id));
    for c in d.entities {
        let i = match entities.iter().position(|e| e.id == c.id) {
            Some(i) => i,
            None => {
                entities.push(Entity { id: c.id, ..Default::default() });
                entities.len() - 1
            }
        };
        let e = &mut entities[i];
        (e.pos, e.vel) = (c.pos.unwrap_or(e.pos), c.vel.unwrap_or(e.vel));
        (e.yaw, e.pitch, e.teleport) = (c.yaw.unwrap_or(e.yaw), c.pitch.unwrap_or(e.pitch), c.teleport.unwrap_or(e.teleport));
        if let Some(p) = c.data {
            e.data = p.apply(&e.data)?;
        }
    }
    let state = match d.state {
        Some(p) => p.apply(&state)?,
        None => state,
    };
    Some(Snapshot {
        tick: d.tick,
        map: d.map.unwrap_or(map),
        you: d.you,
        ack: d.ack,
        echo: d.echo,
        echo_held: d.echo_held,
        state,
        entities,
        events: d.events,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ServerGame, encode};

    fn snapshot(tick: u32, frame: &sdk::Frame) -> Snapshot {
        let (map, state, entities) = (frame.map.clone(), frame.state.clone(), frame.entities.clone());
        Snapshot { tick, map, you: 1, ack: tick, echo: 1.5, echo_held: 0.01, state, entities, events: vec![vec![tick as u8]] }
    }

    /// A 16v16 bot game: every delta rebuilds the exact snapshot, and is much smaller than a whole one.
    #[test]
    fn deltas_rebuild_snapshots_and_save_most_of_the_bytes() {
        let mut game = dust::game::Dust::new();
        game.connect("tester");
        let mut snaps = Vec::new();
        for tick in 1..=(30.0 * sdk::TICK_HZ) as u32 {
            let frame = game.tick();
            if tick % SNAPSHOT_EVERY == 0 {
                snaps.push(snapshot(tick, &frame));
            }
        }
        let (mut whole, mut delta) = (0, 0);
        // Relative to the one three snapshots back: about what a client 100 ms away has acked.
        for pair in snaps.windows(4) {
            let (base, now) = (&pair[0], &pair[3]);
            let d = diff(Some(base), now);
            (whole, delta) = (whole + encode(&diff(None, now)).len(), delta + encode(&d).len());
            assert_eq!(apply(Some(base), d.clone()).as_ref(), Some(now), "rebuilds exactly");
            assert_eq!(apply(None, d), None, "needs its base");
        }
        let n = snaps.len() - 3;
        println!("whole snapshot {} B, delta {} B on average", whole / n, delta / n);
        assert!(delta * 2 < whole, "deltas save at least half");
        assert_eq!(apply(None, diff(None, &snaps[5])).as_ref(), Some(&snaps[5]), "whole snapshots stand alone");
    }

    #[test]
    fn entities_come_and_go() {
        let e = |id: u32, x: f32| Entity { id, pos: Vec3::new(x, 0.0, 0.0), data: vec![id as u8; 40], ..Default::default() };
        let s = |tick: u32, entities: Vec<Entity>| Snapshot { tick, map: "m".into(), you: 1, ack: 0, echo: 0.0, echo_held: 0.0, state: vec![1, 2, 3], entities, events: vec![] };
        let (a, b) = (s(4, vec![e(1, 0.0), e(2, 0.0), e(3, 0.0)]), s(8, vec![e(1, 1.0), e(3, 0.0), e(4, 2.0)]));
        let d = diff(Some(&a), &b);
        assert_eq!((d.removed.clone(), d.entities.len(), d.state.clone(), d.map.clone()), (vec![2], 2, None, None), "1 moved, 4 new, 3 unchanged");
        assert_eq!(apply(Some(&a), d).unwrap().entities.len(), 3);
        let mut c = b.clone();
        c.entities[0].data[7] = 99;
        assert!(matches!(diff(Some(&b), &c).entities[0].data, Some(Patch::Runs(ref r)) if r == &vec![(7, vec![99])]), "one byte patched");
    }
}

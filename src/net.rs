//! What the host sends over the wire. Game data rides inside as opaque bytes.

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
    Snapshot(Snapshot),
    /// The game this server runs: the sha256 and size of its module. `cookie` must come back with chunk requests, so
    /// only the real sender of a hello can make the server send chunks.
    Game { hash: [u8; 32], size: u32, cookie: u64 },
    Chunk { index: u32, data: Vec<u8> },
    /// For server browsers: who we are and what's on, with the query's `time` echoed to measure ping.
    Info { name: String, game: GameInfo, time: f64 },
}

/// Ports server browsers ask on: the default and the next few, for several servers on one machine.
pub const DISCOVERY_PORTS: std::ops::RangeInclusive<u16> = 5000..=5009;
/// An info query must be at least this big; names in the answer are cut to `NAME_LEN` so it's never bigger.
pub const INFO_QUERY_SIZE: usize = 128;
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

#[derive(Serialize, Deserialize, Debug)]
pub struct Snapshot {
    pub tick: u32,
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
    /// The most recent unacked inputs, oldest first. Redundant on purpose to survive packet loss.
    pub inputs: Vec<InputMsg>,
}

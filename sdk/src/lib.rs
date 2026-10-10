//! The boundary between the fps16 host and a game. A game implements `ServerGame` and calls `export_server!` so the
//! host can run it natively (linked in) or as a downloaded `.wasm` module.
//!
//! The wasm ABI is a single shared byte buffer: the host asks for `fps16_buf(len)`, writes its argument there and
//! calls the export with the length; exports that return bytes leave them in the buffer and return `ptr << 32 | len`.

use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const TICK_HZ: f64 = 128.0;

/// Something in the world the host can move without understanding it: it interpolates the pose for remote viewers,
/// smooths prediction corrections with it, and the game's lag compensation rewinds it. The rest is game data.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Entity {
    pub id: u32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Bumped by the game whenever the entity jumps (a respawn), so nobody draws or rewinds across the jump.
    pub teleport: u8,
    pub data: Vec<u8>,
}

/// Something that happened, sent once to `to` (everyone when None).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Event {
    pub to: Option<Vec<u32>>,
    pub data: Vec<u8>,
}

/// The game world after a tick: what the host snapshots for clients.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Frame {
    /// Game-wide state (score, round, timers), sent as is.
    pub state: Vec<u8>,
    pub entities: Vec<Entity>,
    /// Events from this tick only.
    pub events: Vec<Event>,
}

/// The server half of a game. All game data crosses as bytes the game defines; the host only moves them.
pub trait ServerGame {
    /// A player joined; returns the id the game gave them.
    fn connect(&mut self, name: &str) -> u32;
    fn disconnect(&mut self, id: u32);
    /// The player's network round trip in ms, as their client measures it.
    fn ping(&mut self, id: u32, ms: u16);
    /// One input from a player, applied on the next `tick`. `view_tick` is the (fractional) server tick the client
    /// was showing remote entities at, for lag compensation.
    fn input(&mut self, id: u32, seq: u32, view_tick: f32, input: &[u8]);
    /// Advances the game by one tick (1 / `TICK_HZ` seconds).
    fn tick(&mut self) -> Frame;
}

/// Recent entity positions, for lag compensation: rewinding others to where a shooter saw them.
pub struct History {
    ticks: VecDeque<(u32, Vec<(u32, Vec3, u8)>)>,
    keep: usize,
}

impl History {
    pub fn new(keep_ticks: usize) -> History {
        History { ticks: VecDeque::new(), keep: keep_ticks }
    }

    pub fn record(&mut self, tick: u32, entities: &[Entity]) {
        self.ticks.push_back((tick, entities.iter().map(|e| (e.id, e.pos, e.teleport)).collect()));
        while self.ticks.len() > self.keep {
            self.ticks.pop_front();
        }
    }

    /// Where `now` was at a fractional tick: its current position when there's no record, None if it has teleported
    /// since (a respawned player can't be hit where they died).
    pub fn rewind(&self, now: &Entity, at: f32) -> Option<Vec3> {
        let find = |t: u32| {
            let (_, es) = self.ticks.iter().find(|(ht, _)| *ht == t)?;
            es.iter().find(|(id, ..)| *id == now.id).map(|&(_, pos, teleport)| (pos, teleport == now.teleport))
        };
        match (find(at.floor() as u32), find(at.ceil() as u32)) {
            (Some(a), Some(b)) => (a.1 && b.1).then(|| a.0.lerp(b.0, at.fract())),
            (Some(a), None) | (None, Some(a)) => a.1.then_some(a.0),
            (None, None) => Some(now.pos),
        }
    }
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    postcard::to_stdvec(v).expect("serialize")
}

pub fn decode<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Option<T> {
    postcard::from_bytes(b).ok()
}

/// Prints a line on the host's console.
pub fn log(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        #[link(wasm_import_module = "fps16")]
        unsafe extern "C" {
            fn log(ptr: *const u8, len: usize);
        }
        unsafe { log(msg.as_ptr(), msg.len()) }
    }
    #[cfg(not(target_arch = "wasm32"))]
    println!("{msg}");
}

/// Exports a `ServerGame` from a wasm module, made by `$new` on first use.
#[macro_export]
macro_rules! export_server {
    ($new:expr) => {
        #[cfg(target_arch = "wasm32")]
        mod fps16_exports {
            use super::*;
            use std::cell::RefCell;
            use $crate::ServerGame;

            thread_local! {
                static GAME: RefCell<Box<dyn ServerGame>> = RefCell::new({
                    std::panic::set_hook(Box::new(|info| $crate::log(&info.to_string())));
                    Box::new($new)
                });
                static BUF: RefCell<Vec<u8>> = RefCell::new(Vec::new());
            }

            fn game<R>(f: impl FnOnce(&mut dyn ServerGame) -> R) -> R {
                GAME.with_borrow_mut(|g| f(g.as_mut()))
            }

            fn arg<R>(len: usize, f: impl FnOnce(&[u8]) -> R) -> R {
                BUF.with_borrow(|b| f(&b[..len]))
            }

            fn ret(bytes: Vec<u8>) -> u64 {
                BUF.with_borrow_mut(|b| {
                    *b = bytes;
                    (b.as_ptr() as u64) << 32 | b.len() as u64
                })
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_buf(len: usize) -> *mut u8 {
                BUF.with_borrow_mut(|b| {
                    b.resize(len, 0);
                    b.as_mut_ptr()
                })
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_connect(len: usize) -> u32 {
                let name = arg(len, |b| String::from_utf8_lossy(b).into_owned());
                game(|g| g.connect(&name))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_disconnect(id: u32) {
                game(|g| g.disconnect(id))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_ping(id: u32, ms: u32) {
                game(|g| g.ping(id, ms as u16))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_input(id: u32, seq: u32, view_tick: f32, len: usize) {
                let input = arg(len, <[u8]>::to_vec);
                game(|g| g.input(id, seq, view_tick, &input))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_tick() -> u64 {
                ret($crate::encode(&game(|g| g.tick())))
            }
        }
    };
}

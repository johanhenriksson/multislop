//! Runs a game compiled to wasm (see `sdk`) in a sandbox: no imports but `log`, a memory cap, and a time limit per
//! call so a broken game can't hang the server or client.

use sdk::client::{ClientGame, ClientSnapshot, Command, FrameIn, FrameOut, RawInput};
use sdk::{Entity, Frame, ServerGame, decode, encode};
use std::time::Duration;
use wasmtime::{Caller, Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, WasmParams, WasmResults};

const MAX_MEMORY: usize = 256 << 20;
const EPOCH: Duration = Duration::from_millis(10);
/// Epochs one call may run before it's stopped.
const CALL_EPOCHS: u64 = 100;

/// A loaded module and the shared buffer its exports take arguments in and leave results in.
struct Wasm {
    store: Store<StoreLimits>,
    instance: Instance,
    memory: Memory,
    buf: TypedFunc<u32, u32>,
}

impl Wasm {
    fn load(bytes: &[u8]) -> wasmtime::Result<Wasm> {
        let engine = Engine::new(Config::new().epoch_interruption(true))?;
        let module = Module::new(&engine, bytes)?;
        let mut linker = Linker::new(&engine);
        linker.func_wrap("fps16", "log", |mut caller: Caller<'_, StoreLimits>, ptr: u32, len: u32| {
            let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) else { return };
            let data = memory.data(&caller);
            let bytes = data.get(ptr as usize..(ptr as usize).saturating_add(len as usize)).unwrap_or_default();
            println!("{}", String::from_utf8_lossy(bytes));
        })?;
        let mut store = Store::new(&engine, StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build());
        store.limiter(|limits| limits);
        store.set_epoch_deadline(CALL_EPOCHS);
        let ticker = engine.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(EPOCH);
                ticker.increment_epoch();
            }
        });
        let instance = linker.instantiate(&mut store, &module)?;
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| wasmtime::Error::msg("no memory export"))?;
        let buf = instance.get_typed_func(&mut store, "fps16_buf")?;
        Ok(Wasm { store, instance, memory, buf })
    }

    fn func<P: WasmParams, R: WasmResults>(&mut self, name: &str) -> wasmtime::Result<TypedFunc<P, R>> {
        self.instance.get_typed_func(&mut self.store, name)
    }

    fn call<P: WasmParams, R: WasmResults>(&mut self, f: &TypedFunc<P, R>, params: P) -> R {
        self.store.set_epoch_deadline(CALL_EPOCHS);
        // ponytail: a trapped game is left in an unknown state, so the process stops. Reload the module if that's too harsh.
        f.call(&mut self.store, params).unwrap_or_else(|e| panic!("game module failed: {e:?}"))
    }

    /// Copies `bytes` into the argument buffer and returns their length.
    fn arg(&mut self, bytes: &[u8]) -> u32 {
        let buf = self.buf.clone();
        let ptr = self.call(&buf, bytes.len() as u32) as usize;
        self.memory.data_mut(&mut self.store)[ptr..ptr + bytes.len()].copy_from_slice(bytes);
        bytes.len() as u32
    }

    /// The bytes a call returned as `ptr << 32 | len`.
    fn ret(&self, out: u64) -> &[u8] {
        let (ptr, len) = ((out >> 32) as usize, out as u32 as usize);
        &self.memory.data(&self.store)[ptr..ptr + len]
    }

    /// Calls a bytes-in, bytes-out export.
    fn bytes(&mut self, f: &TypedFunc<u32, u64>, arg: &[u8]) -> Vec<u8> {
        let len = self.arg(arg);
        let out = self.call(f, len);
        self.ret(out).to_vec()
    }
}

pub struct WasmGame {
    wasm: Wasm,
    connect: TypedFunc<u32, u32>,
    disconnect: TypedFunc<u32, ()>,
    ping: TypedFunc<(u32, u32), ()>,
    input: TypedFunc<(u32, u32, f32, u32), ()>,
    tick: TypedFunc<(), u64>,
}

impl WasmGame {
    pub fn load(bytes: &[u8]) -> wasmtime::Result<WasmGame> {
        let mut wasm = Wasm::load(bytes)?;
        Ok(WasmGame {
            connect: wasm.func("fps16_connect")?,
            disconnect: wasm.func("fps16_disconnect")?,
            ping: wasm.func("fps16_ping")?,
            input: wasm.func("fps16_input")?,
            tick: wasm.func("fps16_tick")?,
            wasm,
        })
    }
}

impl ServerGame for WasmGame {
    fn connect(&mut self, name: &str) -> u32 {
        let len = self.wasm.arg(name.as_bytes());
        self.wasm.call(&self.connect, len)
    }

    fn disconnect(&mut self, id: u32) {
        self.wasm.call(&self.disconnect, id)
    }

    fn ping(&mut self, id: u32, ms: u16) {
        self.wasm.call(&self.ping, (id, ms as u32))
    }

    fn input(&mut self, id: u32, seq: u32, view_tick: f32, input: &[u8]) {
        let len = self.wasm.arg(input);
        self.wasm.call(&self.input, (id, seq, view_tick, len))
    }

    fn tick(&mut self) -> Frame {
        let out = self.wasm.call(&self.tick, ());
        decode(self.wasm.ret(out)).unwrap_or_default()
    }
}

pub struct WasmClient {
    wasm: Wasm,
    start: TypedFunc<u32, u64>,
    snapshot: TypedFunc<u32, u64>,
    input: TypedFunc<u32, u64>,
    predict: TypedFunc<u32, u64>,
    frame: TypedFunc<u32, u64>,
}

impl WasmClient {
    pub fn load(bytes: &[u8]) -> wasmtime::Result<WasmClient> {
        let mut wasm = Wasm::load(bytes)?;
        Ok(WasmClient {
            start: wasm.func("fps16_client_start")?,
            snapshot: wasm.func("fps16_client_snapshot")?,
            input: wasm.func("fps16_client_input")?,
            predict: wasm.func("fps16_client_predict")?,
            frame: wasm.func("fps16_client_frame")?,
            wasm,
        })
    }
}

impl ClientGame for WasmClient {
    fn start(&mut self) -> Vec<Command> {
        decode(&self.wasm.bytes(&self.start, &[])).unwrap_or_default()
    }

    fn snapshot(&mut self, snap: &ClientSnapshot) {
        self.wasm.bytes(&self.snapshot, &encode(snap));
    }

    fn input(&mut self, raw: &RawInput) -> Vec<u8> {
        self.wasm.bytes(&self.input, &encode(raw))
    }

    fn predict(&mut self, me: &mut Entity, input: &[u8], seq: u32, fresh: bool) {
        let out = self.wasm.bytes(&self.predict, &encode(&(&*me, input, seq, fresh)));
        if let Some(e) = decode(&out) {
            *me = e;
        }
    }

    fn frame(&mut self, f: &FrameIn) -> FrameOut {
        decode(&self.wasm.bytes(&self.frame, &encode(f))).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dust::shared::{NetPlayer, Round};

    /// Builds dust to wasm and plays a bot-filled minute through the module, next to the natively linked game.
    #[test]
    fn dust_runs_as_wasm() {
        let target = concat!(env!("CARGO_MANIFEST_DIR"), "/target");
        let built = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "dust", "--release", "--target", "wasm32-unknown-unknown", "--target-dir", target])
            .status()
            .unwrap();
        assert!(built.success());
        let bytes = std::fs::read(format!("{target}/wasm32-unknown-unknown/release/dust.wasm")).unwrap();
        let mut wasm = WasmGame::load(&bytes).unwrap();
        let mut native = dust::game::Dust::new();
        let games: [&mut dyn ServerGame; 2] = [&mut wasm, &mut native];
        let mut last = Vec::new();
        for g in games {
            let id = g.connect("tester");
            g.ping(id, 30);
            let mut frame = Frame::default();
            for _ in 0..(60.0 * sdk::TICK_HZ) as u32 {
                frame = g.tick();
            }
            let players: Vec<NetPlayer> = frame.entities.iter().filter_map(NetPlayer::from_entity).collect();
            assert_eq!(players.len(), 12, "6v6 with bots");
            assert!(players.iter().any(|p| p.id == id && p.name == "tester"));
            assert!(players.iter().map(|p| p.kills).sum::<u32>() > 0, "bots fought");
            let round: Round = sdk::decode(&frame.state).unwrap();
            println!("round {} score {:?}", round.number, round.score);
            last.push(frame.entities);
        }
        println!("wasm and native {}", if last[0] == last[1] { "match exactly" } else { "diverged" });
    }
}

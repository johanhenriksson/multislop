//! Runs a game compiled to wasm (see `sdk`) in a sandbox: no imports but `log`, a memory cap, and a time limit per
//! call so a broken game can't hang the server.

use sdk::{Frame, ServerGame};
use std::time::Duration;
use wasmtime::{Caller, Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

const MAX_MEMORY: usize = 256 << 20;
const EPOCH: Duration = Duration::from_millis(10);
/// Epochs one call may run before it's stopped.
const CALL_EPOCHS: u64 = 100;

pub struct WasmGame {
    store: Store<StoreLimits>,
    memory: Memory,
    buf: TypedFunc<u32, u32>,
    connect: TypedFunc<u32, u32>,
    disconnect: TypedFunc<u32, ()>,
    ping: TypedFunc<(u32, u32), ()>,
    input: TypedFunc<(u32, u32, f32, u32), ()>,
    tick: TypedFunc<(), u64>,
}

impl WasmGame {
    pub fn load(bytes: &[u8]) -> wasmtime::Result<WasmGame> {
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
        Ok(WasmGame {
            buf: instance.get_typed_func(&mut store, "fps16_buf")?,
            connect: instance.get_typed_func(&mut store, "fps16_connect")?,
            disconnect: instance.get_typed_func(&mut store, "fps16_disconnect")?,
            ping: instance.get_typed_func(&mut store, "fps16_ping")?,
            input: instance.get_typed_func(&mut store, "fps16_input")?,
            tick: instance.get_typed_func(&mut store, "fps16_tick")?,
            store,
            memory,
        })
    }

    /// Copies `bytes` into the game's argument buffer and returns their length.
    fn arg(&mut self, bytes: &[u8]) -> u32 {
        let ptr = call(&mut self.store, &self.buf, bytes.len() as u32) as usize;
        self.memory.data_mut(&mut self.store)[ptr..ptr + bytes.len()].copy_from_slice(bytes);
        bytes.len() as u32
    }

}

fn call<P: wasmtime::WasmParams, R: wasmtime::WasmResults>(store: &mut Store<StoreLimits>, f: &TypedFunc<P, R>, params: P) -> R {
    store.set_epoch_deadline(CALL_EPOCHS);
    // ponytail: a trapped game is left in an unknown state, so the server stops. Reload the module if that's too harsh.
    f.call(store, params).unwrap_or_else(|e| panic!("game module failed: {e:?}"))
}

impl ServerGame for WasmGame {
    fn connect(&mut self, name: &str) -> u32 {
        let len = self.arg(name.as_bytes());
        call(&mut self.store, &self.connect, len)
    }

    fn disconnect(&mut self, id: u32) {
        call(&mut self.store, &self.disconnect, id)
    }

    fn ping(&mut self, id: u32, ms: u16) {
        call(&mut self.store, &self.ping, (id, ms as u32))
    }

    fn input(&mut self, id: u32, seq: u32, view_tick: f32, input: &[u8]) {
        let len = self.arg(input);
        call(&mut self.store, &self.input, (id, seq, view_tick, len))
    }

    fn tick(&mut self) -> Frame {
        let out = call(&mut self.store, &self.tick, ());
        let (ptr, len) = ((out >> 32) as usize, out as u32 as usize);
        sdk::decode(&self.memory.data(&self.store)[ptr..ptr + len]).unwrap_or_default()
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

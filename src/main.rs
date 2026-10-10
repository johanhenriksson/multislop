mod client;
mod model;
mod net;
mod netclient;
mod server;
mod sfx;
mod textures;
mod wasm;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    match args.first().map(String::as_str) {
        Some("server") => {
            let port = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(5000);
            let game: Box<dyn sdk::ServerGame> = match flag("--game") {
                Some(path) => {
                    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
                    Box::new(wasm::WasmGame::load(&bytes).unwrap_or_else(|e| panic!("can't load {path}: {e:?}")))
                }
                None => Box::new(dust::game::Dust::new()),
            };
            server::run(port, game)
        }
        Some("client") => {
            let addr = args.get(1).map_or("127.0.0.1:5000", String::as_str);
            let addr = std::net::ToSocketAddrs::to_socket_addrs(addr).expect("bad address").next().unwrap();
            let num = |name| flag(name).and_then(|v| v.parse().ok());
            client::run(addr, num("--lag").unwrap_or(0), num("--jitter").unwrap_or(0), num("--interp").unwrap_or(50));
        }
        _ => eprintln!(
            "usage: fps16 server [port] [--game game.wasm] | fps16 client [host:port] [--lag ms] [--jitter ms] [--interp ms]"
        ),
    }
}

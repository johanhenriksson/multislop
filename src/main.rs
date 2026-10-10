mod client;
mod net;
mod netclient;
mod render;
mod server;
mod wasm;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    // --game picks the game module: what a server runs (default: dust.wasm next to this program), or on a client one to
    // use instead of downloading the server's.
    let module = |default: Option<std::path::PathBuf>| {
        let path = flag("--game").map(std::path::PathBuf::from).or(default)?;
        Some(std::fs::read(&path).unwrap_or_else(|e| panic!("can't read the game {}: {e} (pass --game file.wasm)", path.display())))
    };
    match args.first().map(String::as_str) {
        Some("server") => {
            let port = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(5000);
            let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("dust.wasm")));
            let module = module(beside).expect("no game");
            let game = wasm::WasmGame::load(&module).unwrap_or_else(|e| panic!("can't load the game: {e:?}"));
            server::run(port, Box::new(game), module)
        }
        Some("client") => {
            let addr = args.get(1).map_or("127.0.0.1:5000", String::as_str);
            let addr = std::net::ToSocketAddrs::to_socket_addrs(addr).expect("bad address").next().unwrap();
            let num = |name| flag(name).and_then(|v| v.parse().ok());
            let game = module(None).map(|bytes| -> client::Game {
                Box::new(wasm::WasmClient::load(&bytes).unwrap_or_else(|e| panic!("can't load the game: {e:?}")))
            });
            client::run(addr, game, num("--lag").unwrap_or(0), num("--jitter").unwrap_or(0), num("--interp").unwrap_or(50));
        }
        _ => eprintln!(
            "usage: fps16 server [port] [--game game.wasm] | fps16 client [host:port] [--game game.wasm] [--lag ms] [--jitter ms] [--interp ms]"
        ),
    }
}

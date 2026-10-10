mod browser;
mod client;
mod net;
mod netclient;
mod render;
mod server;
mod wasm;

/// dust.wasm next to this program, where releases put it.
fn beside() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("dust.wasm"))
}

fn user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "someone".into())
}

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
            let module = module(beside()).expect("no game");
            let mut game = wasm::WasmGame::load(&module).unwrap_or_else(|e| panic!("can't load the game: {e:?}"));
            // --map picks one of the game's maps, its first by default.
            let maps = sdk::ServerGame::info(&mut game).maps;
            let map = flag("--map").cloned().or_else(|| maps.first().cloned()).unwrap_or_default();
            sdk::ServerGame::set_map(&mut game, &map);
            let name = flag("--name").cloned().unwrap_or(format!("{}'s server", user()));
            let sock = std::net::UdpSocket::bind(("0.0.0.0", port)).unwrap_or_else(|e| panic!("can't use port {port}: {e}"));
            server::run(sock, &name, Box::new(game), module)
        }
        Some("client") => {
            // No address: pick a server from the LAN browser.
            let addr = args.get(1).filter(|a| !a.starts_with("--")).map(|a| crate::browser::resolve(a).expect("bad address"));
            let num = |name| flag(name).and_then(|v| v.parse().ok());
            let local = module(None);
            // What "Host a LAN game" in the browser offers: --game, modules next to this program, downloaded ones.
            let mut hostable: Vec<std::path::PathBuf> = flag("--game").map(Into::into).into_iter().collect();
            for dir in [beside().and_then(|p| Some(p.parent()?.to_path_buf())), Some(client::games_dir())].into_iter().flatten() {
                let wasm = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path());
                hostable.extend(wasm.filter(|p| p.extension().is_some_and(|e| e == "wasm")));
            }
            client::run(addr, local, hostable, num("--lag").unwrap_or(0), num("--jitter").unwrap_or(0), num("--interp").unwrap_or(50));
        }
        _ => eprintln!(
            "usage: fps16 server [port] [--game game.wasm] [--map map] [--name name] | fps16 client [host:port] [--game game.wasm] [--lag ms] [--jitter ms] [--interp ms]"
        ),
    }
}

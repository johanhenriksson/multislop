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

/// Game modules in the folder next to this program, by name.
fn modules_beside() -> Vec<std::path::PathBuf> {
    let Some(dir) = beside().and_then(|p| Some(p.parent()?.to_path_buf())) else { return vec![] };
    let mut wasm: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "wasm")).collect();
    wasm.sort();
    wasm
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
            // --rotate plays games in turn: every --game given (it can be repeated), or else every module next to
            // this program. Without it, the first --game only.
            let mut paths: Vec<std::path::PathBuf> = args.windows(2).filter(|w| w[0] == "--game").map(|w| w[1].clone().into()).collect();
            if !args.iter().any(|a| a == "--rotate") {
                paths.truncate(1);
            } else if paths.is_empty() {
                paths = modules_beside();
            }
            if paths.is_empty() {
                paths.extend(beside());
            }
            let modules = paths.iter().map(|p| std::fs::read(p).unwrap_or_else(|e| panic!("can't read the game {}: {e} (pass --game file.wasm)", p.display()))).collect();
            // --map picks one of the (first) game's maps, its first by default; --bots plays with that many bots
            // instead of the game's default.
            let bots = flag("--bots").map(|b| b.parse().expect("--bots takes a number"));
            let games = server::load(modules, flag("--map").map(String::as_str), bots).unwrap_or_else(|e| panic!("{e}"));
            let name = flag("--name").cloned().unwrap_or(format!("{}'s server", user()));
            let sock = std::net::UdpSocket::bind(("0.0.0.0", port)).unwrap_or_else(|e| panic!("can't use port {port}: {e}"));
            server::run(sock, &name, games)
        }
        Some("client") => {
            // No address: pick a server from the LAN browser.
            let addr = args.get(1).filter(|a| !a.starts_with("--")).map(|a| crate::browser::resolve(a).expect("bad address"));
            let num = |name| flag(name).and_then(|v| v.parse().ok());
            let local = module(None);
            // What "Host a LAN game" in the browser offers: --game, modules next to this program, downloaded ones.
            let mut hostable: Vec<std::path::PathBuf> = flag("--game").map(Into::into).into_iter().collect();
            hostable.extend(modules_beside());
            let downloaded = std::fs::read_dir(client::games_dir()).into_iter().flatten().flatten().map(|e| e.path());
            hostable.extend(downloaded.filter(|p| p.extension().is_some_and(|e| e == "wasm")));
            client::run(addr, local, hostable, num("--lag").unwrap_or(0), num("--jitter").unwrap_or(0), num("--interp").unwrap_or(50));
        }
        _ => eprintln!(
            "usage: fps16 server [port] [--game game.wasm]... [--rotate] [--map map] [--name name] [--bots n] | fps16 client [host:port] [--game game.wasm] [--lag ms] [--jitter ms] [--interp ms]"
        ),
    }
}

mod bot;
mod client;
mod map;
mod model;
mod server;
mod sfx;
mod shared;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("server") => server::run(args.get(1).and_then(|p| p.parse().ok()).unwrap_or(5000)),
        Some("client") => {
            let addr = args.get(1).map_or("127.0.0.1:5000", String::as_str);
            let addr = std::net::ToSocketAddrs::to_socket_addrs(addr).expect("bad address").next().unwrap();
            let flag = |name| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)?.parse().ok());
            client::run(addr, flag("--lag").unwrap_or(0), flag("--jitter").unwrap_or(0), flag("--interp").unwrap_or(50));
        }
        _ => eprintln!("usage: fps16 server [port] | fps16 client [host:port] [--lag ms] [--jitter ms] [--interp ms]"),
    }
}

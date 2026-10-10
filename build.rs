/// Compiles in the build number that `tools/release.sh publish` bumps, so client and server can check they match, and
/// builds the games to wasm next to the host, so `cargo run` finds them.
fn main() {
    println!("cargo:rerun-if-changed=build_number.txt");
    let build = std::fs::read_to_string("build_number.txt").unwrap_or_default();
    let build: u32 = build.trim().parse().unwrap_or(0);
    println!("cargo:rustc-env=FPS16_BUILD={build}");

    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // A target dir of its own: the outer build holds the lock on `target`.
    let target = format!("{root}/target/games");
    println!("cargo:rustc-env=FPS16_DEV_GAMES={target}/wasm32-unknown-unknown/release");
    for dir in ["games", "sdk"] {
        println!("cargo:rerun-if-changed={dir}");
    }
    let mut cargo = std::process::Command::new(std::env::var("CARGO").unwrap());
    cargo.args(["build", "--release", "--target", "wasm32-unknown-unknown", "--target-dir", &target]);
    for game in ["dust", "race", "arena", "rocketball", "minigolf"] {
        cargo.args(["-p", game]);
    }
    cargo.env_remove("CARGO_ENCODED_RUSTFLAGS").env_remove("RUSTFLAGS");
    if !cargo.status().is_ok_and(|s| s.success()) {
        println!("cargo:warning=couldn't build the games to wasm; pass --game file.wasm");
    }
}

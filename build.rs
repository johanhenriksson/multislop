/// Compiles in the build number that `tools/release.sh publish` bumps, so client and server can check they match.
fn main() {
    println!("cargo:rerun-if-changed=build_number.txt");
    let build = std::fs::read_to_string("build_number.txt").unwrap_or_default();
    let build: u32 = build.trim().parse().unwrap_or(0);
    println!("cargo:rustc-env=FPS16_BUILD={build}");
}

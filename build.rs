//! Compiles in the build number that `tools/release.sh publish` bumps, so client and server can check they match,
//! and renders the announcer's voice lines with macOS `say` (silence elsewhere) so the game stays one binary.
use std::path::Path;
use std::process::Command;

/// (file, line) for each announcement.
const VOICE: [(&str, &str); 5] = [
    ("t_win", "Terrorists win."),
    ("ct_win", "Counter-terrorists win."),
    ("planted", "The bomb has been planted."),
    ("defused", "Bomb has been defused."),
    ("go", "Go, go, go!"),
];

fn main() {
    println!("cargo:rerun-if-changed=build_number.txt");
    println!("cargo:rerun-if-changed=build.rs");
    let build = std::fs::read_to_string("build_number.txt").unwrap_or_default();
    let build: u32 = build.trim().parse().unwrap_or(0);
    println!("cargo:rustc-env=FPS16_BUILD={build}");

    let out = std::env::var("OUT_DIR").unwrap();
    for (name, line) in VOICE {
        let path = Path::new(&out).join(format!("{name}.wav"));
        let said = Command::new("say")
            .args(["-v", "Daniel", "-r", "185", "-o"])
            .arg(&path)
            .args(["--file-format=WAVE", "--data-format=LEI16@44100", line])
            .status()
            .is_ok_and(|s| s.success());
        if !said {
            std::fs::write(&path, silent_wav()).unwrap();
        }
    }
}

fn silent_wav() -> Vec<u8> {
    let n: u32 = 441;
    let mut w = Vec::new();
    for chunk in [&b"RIFF"[..], &(36 + n * 2).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(),
        &1u16.to_le_bytes(), &44100u32.to_le_bytes(), &88200u32.to_le_bytes(), &2u16.to_le_bytes(), &16u16.to_le_bytes(),
        b"data", &(n * 2).to_le_bytes()]
    {
        w.extend_from_slice(chunk);
    }
    w.resize(w.len() + n as usize * 2, 0);
    w
}

#!/bin/sh
# Build fps16.app (universal, self-updating) and optionally publish it.
#
#   tools/release.sh            build dist/fps16-mac.zip and the update payload
#   tools/release.sh publish    build, then upload to the VPS
#
# Same scheme as zulkan's package-mac.sh, minus assets: the payload is the gzipped
# binary and the gzipped game modules (dust.wasm, race.wasm, arena.wasm, rocketball.wasm and minigolf.wasm, which servers run
# and send to clients), each with a sha256 file that is its update trigger, uploaded last.
set -eu

PUBLIC="${FPS16_PUBLIC:-http://game.stackpoint.se/fps16}"
HOST="${FPS16_HOST:-root@game.stackpoint.se}"
REMOTE="${FPS16_REMOTE:-/opt/zulkan/dist/fps16}"

cd "$(dirname "$0")/.."

# Each publish is a new build number, compiled into client and server; the server refuses other builds.
if [ "${1:-}" = publish ]; then
    printf '%s\n' $(( $(cat build_number.txt 2>/dev/null || echo 0) + 1 )) > build_number.txt
fi
echo "==> build $(cat build_number.txt)"
out=dist
app="$out/fps16.app"
rm -rf "$app" "$out/fps16-mac.zip"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

echo "==> building"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --target "$t"
done
bin="$app/Contents/Resources/fps16"
lipo -create -output "$bin" target/aarch64-apple-darwin/release/fps16 target/x86_64-apple-darwin/release/fps16
strip "$bin"
# lipo+strip can invalidate the linker signature, and an unsigned arm64 binary is killed on exec.
codesign --force --sign - "$bin"
codesign --verify "$bin"

for game in dust race arena rocketball minigolf; do
    cargo build -p "$game" --release --target wasm32-unknown-unknown
    cp "target/wasm32-unknown-unknown/release/$game.wasm" "$app/Contents/Resources/$game.wasm"
done

for f in fps16 dust.wasm race.wasm arena.wasm rocketball.wasm minigolf.wasm; do
    shasum -a 256 "$app/Contents/Resources/$f" | cut -d' ' -f1 > "$app/Contents/Resources/$f.sha256"
    cp "$app/Contents/Resources/$f.sha256" "$out/$f.sha256"
    gzip -c "$app/Contents/Resources/$f" > "$out/$f.gz"
done

cat > "$app/Contents/MacOS/fps16" <<EOF
#!/bin/sh
# UPDATE_BASE= disables the update check.
: "\${UPDATE_BASE=$PUBLIC}"
EOF
cat >> "$app/Contents/MacOS/fps16" <<'EOF'
set -eu
bundled=$(cd "$(dirname "$0")/../Resources" && pwd)
xattr -dr com.apple.quarantine "$(cd "$(dirname "$0")/../.." && pwd)" 2>/dev/null || true
# The game runs from a writable copy: an app opened from Downloads can be translocated by macOS to a read-only path,
# where updating the binary inside the bundle silently fails.
# ponytail: the bundled files only seed missing copies; a newer re-downloaded app is picked up via the update check.
data="$HOME/Library/Application Support/fps16"
mkdir -p "$data"
for f in fps16 dust.wasm race.wasm arena.wasm rocketball.wasm minigolf.wasm; do
    if [ ! -f "$data/$f" ]; then
        cp "$bundled/$f" "$data/$f"
        cp "$bundled/$f.sha256" "$data/$f.sha256"
        chmod u+w "$data/$f" "$data/$f.sha256"
    fi
done
cd "$data"

if [ ! -t 1 ]; then
    mkdir -p "$HOME/Library/Logs/fps16"
    exec >>"$HOME/Library/Logs/fps16/launch.log" 2>&1
    echo "=== $(date) ==="
fi

# Any failure keeps the current file: a flaky connection must not break the install.
update() {
    [ -n "$UPDATE_BASE" ] || return 0
    published=$(curl -fsS -m 10 "$UPDATE_BASE/$1.sha256" 2>/dev/null) || return 0
    [ -n "$published" ] || return 0
    [ "$published" = "$(cat "$1.sha256" 2>/dev/null || true)" ] && return 0
    echo "==> updating $1 to $published"
    if ! curl -fsS -m 600 "$UPDATE_BASE/$1.gz" | gunzip > "$1.part"; then
        echo "    download failed, keeping current $1" >&2; rm -f "$1.part"; return 0
    fi
    if [ "$(shasum -a 256 "$1.part" | cut -d' ' -f1)" != "$published" ]; then
        echo "    checksum mismatch, keeping current $1"; rm -f "$1.part"; return 0
    fi
    chmod +x "$1.part"
    mv "$1.part" "$1"
    printf '%s\n' "$published" > "$1.sha256"
    echo "==> updated $1"
}
update fps16
update dust.wasm
update race.wasm
update arena.wasm
update rocketball.wasm
update minigolf.wasm

if [ $# -gt 0 ]; then exec ./fps16 "$@"; fi

# The server browser: join a LAN server, type an address, or host a LAN game.
exec ./fps16 client
EOF
chmod +x "$app/Contents/MacOS/fps16"

cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>fps16</string>
    <key>CFBundleIdentifier</key><string>se.stackpoint.fps16</string>
    <key>CFBundleExecutable</key><string>fps16</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

(cd "$out" && ditto -ck --sequesterRsrc --keepParent fps16.app fps16-mac.zip)
du -sh "$out/fps16-mac.zip" "$out/fps16.gz" "$out/dust.wasm.gz" "$out/race.wasm.gz" "$out/arena.wasm.gz" "$out/rocketball.wasm.gz" "$out/minigolf.wasm.gz"

if [ "${1:-}" = publish ]; then
    echo "==> publishing to $HOST:$REMOTE"
    ssh "$HOST" "mkdir -p $REMOTE"
    rsync -az "$out/fps16.gz" "$out/dust.wasm.gz" "$out/race.wasm.gz" "$out/arena.wasm.gz" "$out/rocketball.wasm.gz" "$out/minigolf.wasm.gz" "$out/fps16-mac.zip" "$HOST:$REMOTE/"
    rsync -az "$out/fps16.sha256" "$out/dust.wasm.sha256" "$out/race.wasm.sha256" "$out/arena.wasm.sha256" "$out/rocketball.wasm.sha256" "$out/minigolf.wasm.sha256" "$HOST:$REMOTE/"
    echo "published: $PUBLIC/fps16-mac.zip"
fi

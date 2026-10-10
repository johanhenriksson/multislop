# fps16 — minimal CS 1.6 clone

Deadline: playable before 00:00. One weapon, one simple map.

## Run
- `cargo build -p dust --release --target wasm32-unknown-unknown` builds the game module. The host has no game built
  in: servers run a module and clients download it from the server.
- `cargo run --release -- server [port] [--game game.wasm] [--map map]` (headless, default port 5000; the game defaults to
  `dust.wasm` next to the executable, so in development pass `--game target/wasm32-unknown-unknown/release/dust.wasm`)
- `cargo run --release -- client [host:port] [--game game.wasm] [--lag ms] [--jitter ms] [--interp ms]` (`--game` uses a local module instead of downloading the server's, `--lag`/`--jitter` fake delay and bursty arrival to test netcode, `--interp` is the minimum remote-player buffer, default 50)

## Distribution
- `tools/release.sh` builds `dist/fps16-mac.zip`: a universal, unsigned `fps16.app` with the binary and `dust.wasm`,
  whose launcher self-updates each from `$FPS16_PUBLIC/<file>.sha256` + `<file>.gz`, then opens the server browser.
- `tools/release.sh publish` bumps `build_number.txt` and uploads to the VPS; the sha256 files go last since they are
  the update triggers. The build number is compiled in; the server rejects clients on another build and they are told
  to restart (which updates them).

## Server browser
- `fps16 client` without an address opens the LAN browser: every second it broadcasts an info query (padded to 128
  bytes, so answers are never larger) to ports 5000-5009; servers answer with their name (`--name`, default
  "<user>'s server"), the game's map and player/bot counts (`ServerGame::info`), and the query time echoed for
  ping. Servers on another build answer with their build and are listed grayed out. Entries vanish after 3.5 s of
  silence. Click a server, or arrow keys and Enter, or type an address (remembered in
  `~/Library/Application Support/fps16/host`) and press Enter.
- Back to the browser, with the reason shown, when the server goes quiet for 5 s in a game, never answers for 10 s,
  or runs another build; the game's scene, HUD, sounds and environment are torn down and the mouse is freed.
- "Host a LAN game" lists every map of the game modules it can find (`--game`, `.wasm` files next to the binary,
  downloaded ones in `games/`), e.g. "Counterslop on dust", using only the newest file of each game. The scan runs
  in the background from when the browser opens. The picked one runs as a server on a thread in the client, on the
  first free port of 5000-5009, named "<user>'s game", and the client joins it. Compiled module code is cached in
  `compiled/` by module hash and wasmtime version; `games/` and `compiled/` keep the 4 most recently used files.

## Gameplay
- Players are capsules ("rounded cylinders"): radius 0.4, height 1.8, eye at 1.6.
- Movement: Quake/GoldSrc style (ground friction, accel, air accel, gravity, jump). WASD + mouse, space jumps.
- Ctrl crouches over 0.2 s: 1.0 m tall instead of 1.8, eye at 0.85, 34% speed (silent), 30% less spread; standing up
  needs headroom. Hits use the crouched hull (zones scale with it) as it was at the shooter's view tick; crouched
  players are drawn down on one knee.
- HUD (CS 1.6 style): amber health cross, armor, money, ammo, round timer bottom-center; green dynamic crosshair
  that spreads with movement, air and recoil (gray while reloading); score and round messages top; N toggles the
  net_graph line; Tab shows the CS scoreboard (per team: score, deaths, latency; dead/bomb marked) (ping is network round trip only, via
  a client timestamp the server echoes with its hold time) (ping, worst snapshot gap, correction, interp starvation).

## Weapons and economy (`src/weapons.rs`, CS 1.6 stats)
- Knife (always), Glock (T) / USP (CT) free, Deagle $650, MP5 $1500, AK-47 (T) $2500 / M4A1 (CT) $3100, AWP $4750
  (right-click scope), Kevlar+Helmet $1000, HE grenade $300, Flashbang $200, Rocket Launcher $13000. Buy with B then 1-8 during freeze + 20 s, within 15 m
  of your spawn.
- HE grenade: hold fire to wind up (harder throw the longer, up to 0.6 s), release to throw (up to 25 m/s); it
  bounces and goes off after 2 s, up to 98 damage to enemies in the open within 8.9 m. Survivors keep it.
- Flashbang ($200 each, up to 2, sharing the grenade slot with the HE): thrown like the HE, goes off after 1.5 s
  with a bang and a white burst, blinding everyone within 25 m with a line of sight to it (teammates and thrower
  too) for up to 4 s looking at it, far less facing away; the screen whites out and fades with a ringing tone, and
  blinded bots hold fire with their aim drifting.
- Rocket launcher (primary, 4 + 16 rockets, one every 0.7 s, 3 s reload, 75% move speed): a straight 35 m/s rocket
  that explodes on touching a wall, the floor or an enemy, up to 250 damage within 7 m; smoke trail, launch flash and sound for all,
  fireball and smoke on impact (explosions from grenades too).
- Dropping: G tosses the weapon in hand (not the knife) a few meters forward, with its ammo; the dead drop their
  primary (or pistol) and grenade. Walking over a weapon picks it up if that slot is empty (not your own drop for
  0.6 s). Weapons on the ground are cleared each round, at most 48 at once.
- Slots: 1 primary, 2 pistol, 3 knife, 4 grenade, Q last; R reload (auto when empty), draw delay on switch, semi-auto needs a
  click per shot. Mag + reserve ammo, refilled at round start.
- 100 HP. Damage by zone (head x4, legs x0.75), falls off with distance, armor absorbs per weapon penetration.
- Recoil kicks the view (bullets follow it, so pull down); spread grows with movement, air and recoil. Spread is
  seeded by the input seq so predicted and server bullets match. All weapon state lives in `PlayerState`.
- Money: $800 start, $16000 cap; kill $300 (AWP $100, MP5 $600, knife $1500); win $3250, loss $1400 + $500 per
  consecutive loss (up to 3); plant/defuse $300. Survivors keep guns; the dead restart with the team pistol.
- Each weapon has its own model (`src/model.rs`) and its own synthesized sound (silenced thwip for M4A1/USP, booming
  echoing Deagle, the AWP a recorded sample (`games/dust/assets/sfx/awp.wav`, from the office soundboard), harsh AK crack, light MP5/Glock), plus reload, buy, headshot, hit and hurt sounds.
- Movement speed depends on the weapon in hand: knife 108%, pistols/MP5 100%, M4A1 92%, AK 88%, AWP 84% (60% scoped).
- Player names: the logged-in user (`$USER`); bots are named "BOT Hans" etc.
- HUD: HP/armor/money, weapon + ammo, buy menu, kill feed, red flash when hit. Short tracers (AWP longer).

## Rounds (bomb defusal)
- Teams T and CT, humans auto-balanced on join, friendly fire off. Server-side bots fill both teams to 16v16 (or to the
  bigger human team); they path over the map grid via a random detour point each round (Ts share a random site, some lurk;
  CTs hold spots next to cover that watch the T entrance and move between them, facing the entrance), strafe in
  fights and only shoot once slowed down (counter-strafing), plant/defuse, call out spotted enemies so teammates rotate, and
  only notice enemies in a 120° field of view (callouts too), aim with a reaction delay, limited turn speed and angular aim error (~30% accuracy bot vs bot; `src/bot.rs`
  constants are the difficulty knobs).
- The game is Counterslop; its one map is dust, traced from a top-down overview image into an ASCII grid in `src/map.rs` (2 m cells, flat): B site
  top-left, A bottom-left, CT spawn between them, T spawn at the back on the right (as far from A as from B),
  tunnels through the middle. A stack of tall crates
  across mid keeps T spawn from seeing down it to the CT side (a test checks the line of sight). Sites tinted red.
- Height: a second grid (`DUST2_HEIGHTS`, quarter meters, base 36) gives each cell a floor height: 2 m streets, tunnels
  sunk to 0 with ramps (from the dark areas of the overview), B site and T spawn yard raised 1 m. The floor is
  interpolated between cell corners; you walk up rises under 0.5 m, stay glued going down, and bullets/sight lines
  march against the terrain.
- Round: 5 s freeze (look only, movement frozen via `PlayerState::frozen` so prediction agrees), 1:55 live, 5 s end.
- One random T carries the bomb; it drops where the carrier dies and any T picks it up by walking over it.
- Plant: hold E on a site for 3 s. Defuse: CT holds E within 1.5 m of the bomb for 5 s. Bomb: 35 s, kills within 20 m.
- Wins: eliminate the other team (a planted bomb must still be defused), bomb explodes (T), defused (CT),
  time runs out without a plant (CT).
- Dead players fade to black, then watch a living teammate until the next round. Joining mid-round means waiting.
- HUD: score, timer, round/bomb messages, plant/defuse progress. Bomb beeps speed up; explosion sound and flash.

## Look (`src/textures.rs`, all generated at startup)
- Sandstone block walls (world-mapped so courses line up) with a carved frieze band and a cap ledge; arched wooden
  doors and shutters on walls facing open ground; sand-over-flagstone floor on a terrain mesh (darker in tunnels);
  wooden X-braced and green ammo crates; clouds, two rings of mountains, desert beyond the map, distance haze,
  warm hard sun. First-person weapons are held by gloved hands.
- Combat effects: muzzle flash with a light burst, dust puff and bullet hole where shots land (last 80 holes kept),
  blood on player hits: droplets sprayed mostly out the far side (more for damage, headshots and kills) that splat
  where they land, a big splash on a wall up to 3 m behind, pools spreading under the dead, blood over your own
  screen when you're hit; at most 150 splats, cleared each round. Footsteps for anyone running, by distance; Shift walks slowly and silently.
- Announcer: "Go, go, go!", "Terrorists/Counter-terrorists win", "The bomb has been planted", "Bomb has been defused",
  voiced with ElevenLabs ("Adam") into `assets/voice/` and embedded, radio-filtered, with big centered text; "Enemy
  down" after your kills.
- Team radio: bots call "Enemy spotted!", "Need backup!", "I'm in position.", "Sector clear.", "Cover me!" (with
  cooldowns), players with Z then 1-5 (2 s cooldown); the server sends calls only to the caller's team; clients play
  the voice and show a yellow chat line.
- Controllers (via gilrs): the host maps them onto the keys and mouse games already read, so every game works
  unchanged. Left stick WASD, right stick looks (squared, 1800 counts/s at full tilt), A Space, B Ctrl, X R, Y Q, RT/LT
  mouse buttons, RB E, LB G, left stick click Shift, d-pad 1-4, View Tab, Menu B. Any button captures like a click. In
  the server browser the d-pad picks and A joins; hosting still needs the mouse.
- `FPS16_SHOT=out.png [FPS16_POSE="x,y,z,yaw,pitch"]` renders off-screen with the window hidden, saves a screenshot
  and quits, for checking visuals without a person at the screen.

## Structure
- `games/dust`: the game (movement, weapons, map, bots, rounds, economy, snapshot format). Builds natively and to wasm.
- `sdk`: the host/game boundary. `ServerGame` is what the server host calls; `export_server!` exposes it from wasm
  through one shared byte buffer, with `log` as the only import. A game's world is `Entity`s (id, pos, vel, yaw,
  pitch, a `teleport` counter, opaque data) plus an opaque state blob and audience-addressed events; each tick returns
  them as a `Frame`. `History` is the lag compensation rewind games use with the `view_tick` the host passes in.
  `ability` describes anything a player uses as data (trigger, cost, cycle, accuracy, delivery, payload) and runs
  the predicted part (`Hands`: cooldown, reload, bloom, charge, aim); delivery and payload are resolved by the game.
  Each dust weapon is a `Spec` (price, team, slot, kill reward, move speed) around an `Ability`.
- Maps: games list theirs in `GameInfo::maps`; the host picks one with `ServerGame::set_map` (`--map`, default the
  first), and a game may switch on its own. The current map rides in every `Frame` and snapshot; when it changes the
  client host tears down the world and starts a fresh instance of the client game with `start(map)`.
- `sdk::client`: the client half. `ClientGame` (start, snapshot, input, predict, frame) returns retained commands the
  host applies: assets (textures, meshes, materials, sounds), environment (sky, sun, fog), a scene tree (node 0 is
  the camera), and a HUD of flexbox-like nodes with text, images, click reporting and world-anchored placement;
  plus per-frame camera, sounds, lines and spheres. Input arrives as raw keys and mouse motion; the game binds them.
- `games/dust/src/client`: everything you see and hear in dust (world, models, textures, sounds, effects, HUD),
  with no Bevy dependency; `scene.rs` hands out ids, shares meshes/materials and only resends changed HUD nodes.
- Root crate: the host. Networking (`src/net.rs` envelope, `src/server.rs` transport, input queues, link stats),
  `src/netclient.rs` (clock sync, input stamping and redundancy, prediction/reconciliation through the game's
  `predict` step, entity interpolation, net_graph stats), `src/client.rs` (window, raw input, overlays: connection
  status and net graph, Esc always releases the mouse, screenshot and synthetic-input debug switches),
  `src/render.rs` (commands to Bevy), wasmtime loader (`src/wasm.rs`: memory cap, 1 s per call limit).
- Game distribution: a client without `--game` says hello (padded to 64 bytes) and the server answers which game it
  runs: its module's sha256 and size, plus a cookie (keyed hash of the address).
  Clients load a cached module from `~/Library/Application Support/fps16/games/<sha256>.wasm`, or pull it in 1200
  byte chunks (up to 32 per request, cookie required so the server can't be used to flood a spoofed address),
  verify the hash, cache it and load it. Only then do they send input and join. Modules over 64 MB are refused.

## Netcode (server authoritative, Valve-style)
- Plain UDP, postcard-serialized messages. Fixed tick 128 Hz on both sides; snapshots sent at 32 Hz, input packets at 64 Hz
  (CS 1.6 era rates: fewer packets suit Wi-Fi).
- **Client prediction**: each tick the client samples input, stamps a sequence number, simulates it locally with the
  same shared `step_player` fn the server uses, stores it, and sends the last few unacked inputs (redundancy vs loss).
- **Server**: queues each client's inputs and simulates one per tick (two when more than 2 are queued), so bursty
  uplinks don't make players teleport. Prints per-player loss / worst packet gap / buffer stats every 5 s. Sends a snapshot
  every tick with every player's state + the last input seq it processed for the receiving client.
- **Delta snapshots**: clients report the newest snapshot they have; the server sends each snapshot relative to it
  (only changed entities and fields, game data as patches of changed bytes, state only when it changed), keeping 2 s
  of snapshots to diff against and sending whole ones otherwise. A 16v16 snapshot averages ~785 B instead of ~2.8 KB.
- **Reconciliation**: on snapshot, client resets its own state to the server's and replays all inputs newer than the
  acked seq. Remaining visual error is smoothed out over a few frames.
- **Entity interpolation**: remote players are rendered in the past, interpolated between buffered snapshots. The delay adapts to the worst recent snapshot gap (min `--interp`, max 150 ms); if it still runs dry, players are extrapolated on velocity for up to 100 ms.
- **Lag compensation**: each input carries the server tick the client was viewing (interpolation time). On fire, the
  server rewinds other players to that tick (max 400 ms back) from its position history and raycasts there.

## Out of scope (for now)
Grenades, dropped/pickup weapons, halftime/match end, team switching, map height (ramps, stairs), anti-cheat, encryption, connection handshake/timeouts beyond the basics.

## Instaslop (`games/arena`)
- A second game module: instagib free-for-all on one 80 x 80 m map, "arena" (four-way symmetric: corner balconies up
  stairs, wall-side perches and a central tower reached by jump pads, pillars and low crates). Blocks on a height grid
  in half meters; a 0.5 m rise is walked up as a stair.
- Movement: Quake style, faster (9 m/s) and floatier (gravity 12 m/s², jumps 1.75 m), hold space (or backspace) to keep hopping,
  strafe jumping builds speed. Jump pads throw you ~7 m up, carrying up to 6.5 m/s of your run.
- Railgun only: hitscan, lag-compensated, one-shot frag through everyone along the beam, 1 s recharge (coils on the gun
  and a bar under the crosshair). Beams in the shooter's color; the fragged burst into bouncing gibs.
- Click to respawn after 1 s (automatic after 3 s), at the spawn furthest from the living. First to 25 frags or most
  after 10 min wins; 8 s scoreboard, then a fresh match. Bots fill up to 8 players while anyone is on.
- Run: `cargo build -p arena --release --target wasm32-unknown-unknown`, then
  `cargo run --release -- server --game target/wasm32-unknown-unknown/release/arena.wasm`. Not in releases yet.

## Raceslop (`games/race`)
- A third game module: an obstacle course race over lava, 30 m wide. Two maps: "random" (the default) deals a new
  course every race, and "gauntlet" is the hand-made one, 172 m long: stepping stones, a climb of 1 m blocks, a
  one-cell beam that zigzags, jump pads up onto a ledge, islands across 4 m gaps, and a platform swept by two spinning
  laser bars, then the checkered finish. Same height grid as the arena; 0 is lava.
- Random courses are six of those seven kinds of obstacle (plus pillars to weave through) in random order and shape,
  full-width landings between them, a checkpoint after every second one. The course is named `random-<seed>` and both
  server and client build it from the seed, so nothing but the name goes over the wire. Seeds mix in ping times, so
  courses differ between server runs; a course a bot can't finish is redealt from the next seed.
- Look: a sunset sky dome over slowly drifting lava, basalt spires standing in it, glass walls with posts and a glowing
  rail, platforms colored by height (a color per meter) with glowing edges, green gates over checkpoints, a gold one
  over the finish.
- Movement as in Instaslop (fast, floaty, strong air control), no weapons. Lava or a laser bar sends you back to the
  last of 3 checkpoints; R does it on purpose. The lasers run on the race clock, so client and server agree on them.
- Races: 4 s countdown frozen on the line, then go. Ends when every person is over the line, 30 s after the first
  finisher, or after 5 min; 8 s of standings (place, time, wins), then the next race. Bots fill up to 6 runners and run
  the course along a distance field over walk, jump and pad moves, jumping at ledges and over incoming bars.
- Run: `cargo build -p race --release --target wasm32-unknown-unknown`, then
  `cargo run --release -- server --game target/wasm32-unknown-unknown/release/race.wasm`. Shipped in releases next to
  `dust.wasm`, so it shows up under "Host a LAN game".

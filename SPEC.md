# fps16 — minimal CS 1.6 clone

Deadline: playable before 00:00. One weapon, one simple map.

## Run
- `cargo build -p dust --release --target wasm32-unknown-unknown` builds the game module. The host has no game built
  in: servers run a module and clients download it from the server.
- `cargo run --release -- server [port] [--game game.wasm] [--map map]` (headless, default port 5000; the game defaults to
  `dust.wasm` next to the executable, so in development pass `--game target/wasm32-unknown-unknown/release/dust.wasm`)
  `--rotate` plays games in turn (see Game rotation): every `--game` given (repeat it), or else every module next to
  the executable.
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
- The menu is titled Multislop (so is the window). It shows who you're playing as: the logged-in user (`$USER`). The
  name travels as an `sdk::Profile` in every input packet and reaches the game through `ServerGame::connect`.
- Colors are assigned, not picked: Instaslop and Raceslop give every player and bot joining the first of the 16
  `sdk::PALETTE` colors nobody in the game has (`sdk::free_color`; past 16 players, the least used), so colors are
  unique, and show it on the player's model, names and effects (Counterslop and Rocketball keep team looks).
- Esc in a game (or while loading) opens the Esc menu: Resume (grabs the mouse again), Disconnect (back to the
  browser) and Quit game. The game keeps running underneath but gets no input while it's open; Esc closes it.
- Back to the browser, with the reason shown, when the server goes quiet for 5 s in a game, never answers for 10 s,
  or runs another build; the game's scene, HUD, sounds and environment are torn down and the mouse is freed.
- "Host a LAN game" lists every map of the game modules it can find (`--game`, `.wasm` files next to the binary,
  downloaded ones in `games/`), e.g. "Counterslop on dust", using only the newest file of each game. The scan runs
  in the background from when the browser opens. The picked one runs as a server on a thread in the client, on the
  first free port of 5000-5009, named "<user>'s game", and the client joins it. Its Players setting (and a server's
  `--players n`, `ServerGame::set_min_players`) is a minimum: bots fill up to it while anyone is on and leave as
  people join; "auto" leaves it to the game. Compiled module code is cached in
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
- Teams T and CT, humans auto-balanced on join, friendly fire off. Server-side bots fill both teams to 4v4 (or to the
  bigger human team); they path over the map grid via a random detour point each round (Ts share a random site, some lurk;
  CTs hold spots next to cover that watch the T entrance and move between them, facing the entrance), strafe in
  fights and only shoot once slowed down (counter-strafing), plant/defuse, call out spotted enemies so teammates rotate, and
  only notice enemies in a 120° field of view (callouts too), aim with a reaction delay, limited turn speed and angular aim error (~30% accuracy bot vs bot; `src/bot.rs`
  constants are the difficulty knobs).
- The game is Counterslop, on two maps (`MAPS`, the host picks; `map::load` switches the per-thread current `MAP`).
  Maps are ASCII grids (`games/dust/src/maps/<map>.txt`) with a height grid each (`<map>_heights.txt`, quarter
  meters as base-62 digits 0-9a-zA-Z), in layers separated by a blank line; cell size per map (dust 1 m, aztec 2 m).
  The base layer: `#` wall (height = its top; walls 6.5 m and up get a random 0-3 m extra for the skyline), `x`/`X`
  low/tall crate standing on its height, `k` crate whose top is its height (standing on the lowest floor beside its pile),
  `A`/`B` site floor, `T`/`C` spawn, `.` floor. Upper layers (blank = nothing): `.` (or a site/spawn letter) a floor
  slab 0.6 m thick whose top is the height, `#` a 2 m roof block whose underside is the height. So a cell has a stack
  of floors (`map::Floor`: a surface over a solid underside, the ground solid all the way down), with air between.
- Dust is de_dust2, converted from the original map's geometry (an OBJ export of the CS 1.6 BSP, not in the repo) by
  `tools/dust2_to_grid.py <Dust2.obj> [--debug dir]`: each 1 m cell is sampled 4x4 times with vertical rays; upward
  faces with 1.85 m of air over them are floors, a flood fill from the sites (up a jump, down any drop) keeps the
  reachable ones so roofs drop out. The lowest reachable floor is the base layer, a second one above it a slab, a
  ceiling within 6.5 m a roof block (so tall rooms like the lower tunnels' stairs don't open to the sky); a cell's
  height is the median of its biggest cluster of close sample heights, so a cell half on a ramp and half on a ledge
  gets one of them rather than a step in between. Cells without floor become walls as tall as the faces in them
  (raised over neighboring roofs; ones with no faces of their own but the floor's, inside thick walls, as tall as the
  walls around them rather than flush with the floor) or crates by texture. Heights are shifted up 4 m so the
  lowest floor (the pit at long A) is 0.
  Sites are flood filled around the X decals (A within 7.5 m on the platform, B within 8 m), 16 spawns per team on a
  checkerboard in each spawn yard. `--debug` writes a top-down PNG per layer. The result, 85x89 cells, north up: B
  top-left, A top-right, CT spawn north of mid doors with the B doors corridor west and a passage east to A that runs
  under the bridge of short A (the stacked floor), mid down the middle with catwalk east of it, long A down the east
  side through long doors to the pit, T spawn bottom-left with upper tunnels north to B and lower tunnels into mid.
- Aztec (85x47, one layer, ported from the old pre-wasm counter-slop branch): CT spawn west, A site north-west, B
  south-middle, T spawn east, 16 spawn spots each; the rope bridge is a raised walkway along the north wall of a
  sunken water room, plus sunken grass pits and double doors. Bots play it with the same logic (a test plays 10
  rounds there).
- Floors and movement: a floor's surface is interpolated between its corners, each the average of the floors around
  that corner within the map's ramp height (dust 1.05 m, aztec any), so close heights become slopes and others meet
  at a ledge. You stand on the highest floor at most a step (0.5 m) above your feet, walk up rises under a step, stay
  glued going down, fall off ledges, can't walk into a floor or slab between a step and head height, and bump your
  head on slab undersides (floors only at the hull's center point; boxes as before). Bullets, sight lines, grenades
  and dropped weapons march against the floors, so slabs block them like the ground. Bots path over nodes, one per
  floor of each cell, linked to the highest floor in each neighboring cell at most a step or a ramp up (any drop down)
  with headroom; `Map::nodes` are the ones the T spawn can walk to. No T spawn can see a site or the CT spawn (a test
  checks the line of sight). Sites tinted red.
- Round: 5 s freeze (look only, movement frozen via `PlayerState::frozen` so prediction agrees), 1:55 live, 5 s end.
- Halftime every 10 rounds (`HALF`): teams switch sides with their scores, everyone restarts with the pistol and
  $800 and loss bonuses reset; the next freeze says "teams switched sides".
- First to 11 rounds (`WIN_ROUNDS`) wins the match: after that round's end pause the server moves on to the next map
  (dust, aztec, dust, ...) for a fresh match, and a host rotating games moves on to the next game.
- One random T carries the bomb; it drops where the carrier dies and any T picks it up by walking over it.
- Plant: hold E on a site for 3 s. Defuse: CT holds E within 1.5 m of the bomb for 5 s. Bomb: 35 s, kills within 20 m.
- Wins: eliminate the other team (a planted bomb must still be defused), bomb explodes (T), defused (CT),
  time runs out without a plant (CT).
- Dead players fade to black, then watch a living teammate until the next round. Joining mid-round means waiting.
- HUD: score, timer, round/bomb messages, plant/defuse progress. Bomb beeps speed up; explosion sound and flash.

## Look (`src/textures.rs`, all generated at startup)
- Sandstone block walls (world-mapped so courses line up) with a carved frieze band and a cap ledge; arched wooden
  doors and shutters on walls facing open ground; sand-over-flagstone floor on a terrain mesh (darker under roofs and
  slabs), stone sides on ledges and slab edges, roofs plain stone;
  wooden X-braced and green ammo crates; clouds, two rings of mountains, desert beyond the map, distance haze,
  warm hard sun. First-person weapons are held by gloved hands.
- Aztec swaps in its own look: rough grey stone blocks with grime streaks, moss and hanging vines under a carved key
  frieze; grey paving with grass in the joints, greener and darker down low, murky water at the bottom of the pits;
  damp weathered wood; overcast grey sky, dim cool light, greenish haze, dark green jungle hills and jungle floor.
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
- Game rotation (off by default; `--rotate`, or "Rotate games" in "Host a LAN game", which starts with the picked
  game and then each other one): a game sets `Frame::finished` once its turn is played (dust 20 rounds, race 3
  races, arena 3 matches, rocketball 2 matches, minigolf 1 course, each counted when its end screen is over, reset by `set_map`). The server then disconnects
  everyone from it, starts the next on its first map and bumps a game counter. All modules are loaded up front, and
  each game keeps its own tick (lag compensation rewinds by it). Input for an earlier game is answered with
  `ServerMsg::Game` (hash, size, counter), so clients notice, tear down and load the new game like on connect, then
  rejoin; snapshots carry the counter so a late one from the old game is dropped.
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
Grenades, dropped/pickup weapons, match end, team switching, anti-cheat, encryption, connection handshake/timeouts beyond the basics.

## Instaslop (`games/arena`)
- A second game module: instagib free-for-all on three four-way symmetric maps, each with its own sky, colors and trim:
  "arena" (80 x 80 m, dusk: corner balconies up stairs, wall-side perches and a central tower reached by jump pads,
  pillars and low crates), "foundry" (72 x 72 m, smoky steelworks: the floor at 1 m around a sunken crucible of
  glowing molten metal with a 4 m pillar in it that pads reach, corner furnace towers with catwalks, perches up stairs)
  and "spire" (80 x 80 m, neon night: a stepped ziggurat climbable up all four faces, 5 m corner towers only pads
  reach, ledges up stairs along the walls). Blocks on a height grid in half meters; a 0.5 m rise is walked up as a
  stair. Glowing trim only along real ledges, not where blocks of equal height meet.
- Movement: Quake style, faster (9 m/s) and floatier (gravity 12 m/s², jumps 1.75 m), hold space (or backspace) to keep hopping,
  strafe jumping builds speed. Jump pads throw you ~7 m up, carrying up to 6.5 m/s of your run.
- Railgun only: hitscan, lag-compensated, one-shot frag through everyone along the beam, 1 s recharge (coils on the gun
  and a bar under the crosshair). Beams in the shooter's color; the fragged burst into bouncing gibs.
- Click to respawn after 1 s (automatic after 3 s), at the spawn furthest from the living. First to 25 frags or most
  after 10 min wins; 8 s scoreboard, then a fresh match. Bots fill up to 8 players while anyone is on.
- Run: `cargo build -p arena --release --target wasm32-unknown-unknown`, then
  `cargo run --release -- server --game target/wasm32-unknown-unknown/release/arena.wasm`. Shipped in releases.

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
- Runners: rounded figures built from scaled spheres, a white suit with jersey, sleeves, belt, helmet crest and shoes
  in the player's color, a dark visor. They lean into a run with arms and legs swinging (knees kick up on the back
  swing), tuck with arms up going up, flail coming down and squash on landing.
- Movement: 9 m/s runs, platformer jumps, no weapons. Full speed in ~0.15 s, a stop in ~0.3 s. Gravity 13 m/s² going
  up and 21 coming down; holding space jumps 1.9 m, a tap ~0.6 m (letting go while rising triples gravity). A jump
  still works 0.1 s after running off a ledge, and a press up to 0.125 s before landing jumps on touchdown; held, you
  hop on every landing. Air control: Quake-style sharp corrections and braking plus a slow build towards full speed
  (18 m/s²). Pads throw you 7 m up. The camera eases over step-ups and dips on landing, with a thud. All movement
  state (jump held, coyote and buffer ticks) is in `PlayerState`, so prediction matches the server.
- Lava or a laser bar sends you back to the last of 3 checkpoints; R does it on purpose. The lasers run on the race
  clock, so client and server agree on them.
- Races: 4 s countdown frozen on the line, then go. Ends when every person is over the line, 30 s after the first
  finisher, or after 5 min; 8 s of standings (place, time, wins), then the next race. Bots fill up to 6 runners and run
  the course along a distance field over walk, jump and pad moves, jumping at ledges and over incoming bars, holding
  jump while rising.
- Run: `cargo build -p race --release --target wasm32-unknown-unknown`, then
  `cargo run --release -- server --game target/wasm32-unknown-unknown/release/race.wasm`. Shipped in releases next to
  `dust.wasm`, so it shows up under "Host a LAN game".

## Rocketball (`games/rocketball`)
CS meets Rocket League: two teams of four on a walled pitch with goals at both ends, a giant ball, and rocket
launchers and AWPs. You score by moving the ball into the other goal: rockets blast it, running into it pushes it.
Rockets kill too, so every shot is a choice between the ball and the player guarding it. The AWP makes a second way to
play: hang back and pick off attackers from across the pitch.

### Pitch (one map, "pitch")
- A 2 m cell grid with heights in half meters, like the arena (`#` wall, `.` open, `S` spawn, `J` jump pad, pickups
  by letter). Inside the walls it's 48 m wide (x) by 88 m long (z), and it's the same when rotated 180°, so both halves
  are equal. Orange defends the +z end, Blue the -z end.
- Goals are centered in each end wall: 16 m wide, 8 m tall and 4 m deep. The goal recess has full-height walls on its
  back and sides. Above the mouth is a crossbar, a box spanning it from 8 m up to the wall top, so a high ball
  bounces back into play. Boxes don't have to stand on the floor (they're plain AABBs), so the crossbar is just one
  more box added to the map's boxes.
- Outer walls go up 18 m, and a ceiling at 18 m stops the ball (players can't get that high).
- Cover: chest-high blocks (1.5 m) that hide a running body but not the head, and stop a rolling ball, which has to be
  lofted over them. Tall pillars (4 m) block everything and send the ball off at angles. Raised 2 m platforms along
  both sidelines at midfield have stairs and a jump pad up. Nothing stands within 10 m of the center spot (kickoff),
  and the 16 m in front of each goal is open apart from two pillars, so a shot on goal isn't luck.
- Spawns: 4 kickoff spots per team in its own half (the classic diagonal, two wide, a back, a keeper), plus respawn
  spots near its own goal.

### Movement
- Instaslop's: Quake style, 9 m/s, gravity 12, 1.75 m jumps (space or backspace), strafe jumping, jump pads (copy `games/arena/src/shared.rs`
  movement against the pitch's boxes).
- Explosions throw players: up to 11 m/s at the center, falling off to nothing at the blast radius, away from the
  blast with an upward bias. They throw teammates and yourself as well, so rocket jumps work. Knockback is applied on
  the server only; your own prediction catches up on the next snapshot as a smoothed correction.
- Players never collide with each other or with the ball (the ball gets out of the way instead, see Ball).

### Guns
Everyone carries both: `1` the launcher, `2` the AWP, `Q` the other one. Switching takes 0.6 s, and the gun in hand is
kept through respawns.

#### Rocket launcher
- A straight, 40 m/s rocket, no gravity, one every 0.8 s, 4 to a load, a 2 s reload. It goes off on touching the
  world, any player but the shooter, or the ball, and goes off after 5 s at most.
- Blast: up to 100 damage at the center, falling off linearly to 0 at 4.5 m, through open line of sight only. Friendly
  fire is on at half damage, yourself included (a rocket jump costs about 30 HP), and a teamkill takes a kill off the
  scoreboard.
- Ammo: you spawn with 4 loaded + 8 reserve, and carry at most 16 reserve. When you're out, you're out until you find
  an ammo pickup (or die).
- Shooting uses `sdk::ability` (`Hands`) in the predicted step, like dust's rocket. Rockets are server entities;
  clients draw them from snapshots. The launch sound and flash play predicted.

#### AWP
- Dust's: hitscan, 115 damage (one body shot kills without armor; armor takes its usual 2/3), one shot per press every
  1.45 s, 5 to a magazine, 3 s reload. You spawn with 5 + 10 and carry at most 20 reserve; rocket pickups also give 5.
- Right mouse toggles the scope: dust's black lens overlay, a narrow field of view, slower mouse, and pinpoint accuracy
  (unscoped it's wild, worse moving or in the air). Scoped, you run at 60% speed. The scope drops on reload or switch.
- Hits are lag compensated (players rewound up to 0.4 s to where the shooter saw them). Teammates take half damage,
  as with rockets. A round into the ball sends it on at 10 m/s along the shot and counts as a touch.
- A fading tracer from the muzzle to where the round stopped, and dust's recorded AWP sound.

### Ball
- A sphere, radius 1.2 m, simulated on the server only and sent as an entity, so clients interpolate it like a
  player.
- Gravity 9 m/s², 0.6 of the speed kept on a bounce off the floor, 0.75 off walls, blocks, crossbar and ceiling. It
  rolls with light friction (about 1 m/s² while on the floor) and slight air drag, up to 45 m/s. Collision is sphere
  vs AABB by closest point, so an edge below the center sends it over a step and an edge above sends it back. Swept
  in substeps, so it never tunnels through a wall at full speed.
- Blasts push it: up to 22 m/s added at the center, falling off to 0 at 6 m (measured to the ball's surface, so a
  direct hit is the full push), away from the blast with an upward bias of 0.25 before normalizing. That makes a
  rocket under the ball loft it.
- Touches: a player capsule overlapping the ball pushes it out to touching, and adds 1.5 times the player's approach
  speed along the contact normal, at least 3 m/s (a dribble). A ball flying into a player bounces off them the same
  way. Neither hurts anyone.
- The last team to touch it (push or blast) colors it, and the last player of each team to touch it is remembered for
  goal credit.
- Safety: a ball that ends up non-finite or out of the map is put back on the center spot.

### Pickups
Each sits on a pad on the map, and is taken by walking over it, only when it would help (full health doesn't take
health). It comes back after its respawn time; clients learn whether each is up from the match state.
- `h` health +25 (up to 100), 20 s. `a` light armor +50 (up to 100), 25 s. `r` rockets +8 reserve and AWP rounds
  +5, 15 s.
- The better ones are at midfield on the sideline platforms, where both teams fight over them: `M` mega health +100
  (up to 200, anything over 100 drains 1 HP/s), 35 s; `A` heavy armor +100 (up to 150), 35 s.
- Armor takes 2/3 of damage until it runs out.
- Ammo and small health sit near each team's goal and along the sidelines; light armor on the flanks of each half.

### Match
- Two teams, Orange and Blue. Humans auto-balance on joining. Bots fill each team to 4 while anyone plays (or up to the
  host's player count, keeping the teams even).
- Kickoff: everyone is put on their kickoff spot, frozen for a 3 s countdown (look only), with the ball resting on the
  center spot; then go.
- A goal is scored when the ball's center is a full radius past the goal line, inside the mouth. The ball bursts in the
  scoring team's color, everyone freezes, and 3 s later it's kickoff again. The goal goes to the scoring team's last
  toucher ("X scored!"). If only the defending team touched it, it's an own goal for the scorer's side.
- Clock: 5:00, running only during play. Most goals wins. A tie goes to golden goal overtime for up to 2 minutes,
  then it's a draw. Then 8 s of the scoreboard, and a new match. Rotation: 2 matches per turn.
- Dying: respawn after 5 s, automatically, at a free respawn spot of your team, with 100 HP, no armor, 4 + 8 rockets
  and 5 + 10 AWP rounds.
  Kills score nothing on their own (they buy time), but the scoreboard counts them.
- Scoreboard (Tab, and at the end): per team, the players with goals, kills, deaths and ping.

### Bots
- Roles each tick by distance to the ball: the closest one attacks, the one furthest back keeps (it stays between
  the ball and its own goal, in front of the goal, with the AWP, switching to the launcher to clear a ball within
  20 m of its goal), and the rest support (they trail the attacker by about 12 m).
- Attacking: get behind the ball on the line from the enemy goal through it (3.5 m behind). Once lined up within
  about 30°, either run through it, or from 4-15 m fire a rocket at its lower half, leading its velocity, to blast it
  toward goal. They don't shoot the ball toward their own goal.
- Fighting: an enemy in view (120° field, line of sight) within 25 m who is nearer than the ball shot draws fire at
  their feet, leading their velocity. A keeper with the AWP scopes in and fires at the body of enemies within 70 m.
  Reaction delay, turn speed and aim error are the difficulty knobs, as in the
  arena.
- They go for health under 50 HP and for ammo when out, if a pickup is up and within 25 m. They path over the grid
  with the arena's distance field toward a target cell, recomputed when the target cell changes.

### Look
- A night stadium. Deep blue sky with stars, four floodlight towers at the corners outside the walls (pylons
  topped with banks of glowing lamps, and bright lights over the pitch) and a cool key light casting the shadows.
- Turf with mowing stripes across the pitch every 4 m, and painted white lines: sidelines, halfway line, a 10 m
  center circle and spot, and a box in front of each goal. Each half is faintly tinted toward its team's color.
- Walls: 3 m concrete boards with a glowing strip along the top in the color of the half they're in, and glass above
  up to 18 m on posts every 8 m. Banked stands outside the long walls hold a crowd: thousands of small boxes in team
  colors and dark clothes, all in one mesh.
- Goals: posts and crossbar glowing in the team color, a net across the back and sides (a masked grid texture), and
  a team-colored light inside.
- Cover blocks: concrete with hazard-striped tops and glowing edges in the color of the half they're in. Pillars are
  taller, with a band of light.
- Ball: big, white, with dark panel seams and a glow in the last toucher's team color (white at kickoff). Fast balls
  leave a fading trail. A small world-anchored marker over it tracks it through cover.
- Players: the arena's armored figures in their team's color, carrying a rocket launcher or an AWP (dust's models). Rockets are
  dust's, with a smoke trail and a glow in the team color. Explosions are dust's fireball, smoke and light flash.
- Pickups hover and spin over a glowing pad: health a white box with a red cross (mega a big pulsing one), armor a
  vest-shaped plate (yellow light, red heavy), rockets an olive crate with warheads. A taken pickup leaves the pad dim.
- A goal: a burst in the scoring team's color at the ball (fireball, confetti, a light flash), a horn and a roar from
  the crowd, and a huge "GOAL!" with the scorer's name.
- Sounds: launch and explosion (dust's), a deep thump when the ball is pushed and a metallic clang when it's blasted,
  a goal horn, a crowd ambience loop that swells with play near a goal, pickup sounds (health chime, armor clank, ammo
  clack), countdown beeps, a whistle at kickoff and at the end.
- HUD: a Rocket League style score bug top center (an orange score box, the clock, and a blue score box, with
  "OVERTIME" when it applies), health and armor bottom left, the gun in hand with its load and reserve bottom right, a
  crosshair (the scope when scoped), a kill feed top right, centered messages (kickoff countdown "3, 2, 1, GO!", goals, "Respawning in 2"),
  and the Tab scoreboard.

- Run: `cargo build -p rocketball --release --target wasm32-unknown-unknown`, then
  `cargo run --release -- server --game target/wasm32-unknown-unknown/release/rocketball.wasm`. Shipped in releases next to
  the others.

## Minigolf (`games/minigolf`)
Minigolf in the style of Golf With Your Friends. Everyone plays the same hole at the same time, each at their own
pace: no turns, and balls pass through each other. You see the others as ghosts.

### Course
- One map, "random": a 9-hole course dealt from a seed, named `random-<seed>` like Raceslop's courses, so server and
  client build it from the name and nothing else goes over the wire. Hole `i` comes from the seed and `i`. A new
  course (new seed) every time the last one is over, and when the first person joins, unless the host picked one by
  name (`--map random-42`), which is kept for that course.
- All 9 holes are built at start and laid side by side, 200 m apart along x, floating in the sky, so changing holes
  only moves the camera and nothing is torn down; which hole a ball is on follows from its x. The hole being played
  is in the match state.
- Grid: 1 m cells with floor heights at the cell corners, interpolated in between, so ramps and hills are smooth
  slopes. 0.3 m rails stand wherever the inside (floor and water) meets the void, runs at one height merged into one
  box; under the floor's edge over gaps and water a hidden face stops a ball that drops past it from popping up.
- A hole is dealt by a turtle heading north from a 3-cell tee: 3-6 pieces, a turn (left or right) before some of
  them, then a 2-cell lead-in and a 5 x 5 green, gently tilted, with the cup on one of its middle 9 cells. The path
  is 3 wide; pieces 5 wide get 2-cell lead-ins and lead-outs so they never touch a turn. Pieces:
  - straight (3-8 m), and square 90° turns
  - a ramp up or down 0.5-1 m over 5-7 m, eased at both ends; a hill or a dip of 0.25-0.5 m
  - a slope across the fairway (up to 1 m from side to side) that pulls the ball to one side
  - bumpers: posts scattered over a wide stretch
  - a windmill: a paddle spinning across the fairway around a post, on the ball's own clock (`Ball::clock`, ticks
    since the hole started), so prediction and the server agree on it exactly
  - a gap of 2-3 m over the void after a 3 m run-up and a kicker ramp
  - a split: a wide stretch with a wall of blocks down the middle
  - water: a pool along one side of a wide stretch, or across it with a one-cell bridge
- A hole whose path runs into itself (pieces may only touch the one before them) or whose cup can't be reached over
  the floor and across gaps is dealt again, up to 64 times, then it's a plain straight hole.
- Par: 1 + one per 14 m of path + one per gap, windmill or water crossing, between 2 and 5.
- Three looks, picked from the seed: meadow (green felt, wooden rails, earth sides, blue sky), candy (pink felt, mint
  greens, white rails, wafer sides, pink sky), neon (dark felt, glowing rails, night sky). Every look has a darker tee,
  a lighter green, mowing stripes and clouds below.

### Ball
- A sphere, radius 0.15 m. Simulated by a shared `step_ball`, run by the server and predicted by the client from the
  same inputs, so your own ball moves without lag.
- Gravity 9.8 m/s², rolling friction 1.4 m/s² (a full-power shot rolls about 50 m on the flat), slopes pull it
  downhill, and a slope too gentle to beat friction holds a stopped ball. Bounces keep 0.7 of the speed off rails,
  blocks and the windmill (whose paddle's own motion is added), 0.35 off the floor, nothing when slower than 0.8 m/s.
  Sphere against box by closest point in 5 cm substeps. Top speed 20 m/s.
- At rest when under 0.05 m/s on the floor for 0.25 s; only then can you shoot.
- Cup: radius 0.2 m. A ball over it going under 2 m/s drops in; faster, it rolls on over.
- Out: into the void (2.5 m below the hole's lowest floor) or into water: you see it fall or sink for 0.6 s, then it's
  back where the shot was taken from, +1 stroke.

### Shooting
- Third-person orbit camera around your ball: the mouse turns it (yaw, and pitch between 10° and 70° down), W/S zoom
  between 2 and 10 m. Each hole starts looking north down the tee.
- Hold the left button and pull the mouse back to set power (500 counts for full power, 12 m/s; the view stops
  turning while you pull), release to shoot along the camera's yaw. Right click cancels. An arrow on the ground shows
  direction, growing and reddening with power, and a power bar fills.
- The shot is one input (yaw, power), applied by `step_ball` on the server and in prediction. A shot while rolling,
  frozen or picked up is ignored.
- Max 10 strokes per hole: then you're picked up and score 10.

### Hole flow
- 3 s on the tee, frozen, while the camera flies over the hole from the tee to the cup, with the hole number and par.
- Then everyone plays. Sinking it: a cup rattle, "Birdie!" (or "Par", "Bogey", ...) with your strokes, a cheer under
  par, and a feed line for everyone else; then you watch the others still playing (click cycles who; their ghost
  turns solid for you).
- The hole ends when every person is done (holed or picked up), or 2:30 after it started (the rest score 10). Then
  5 s of the scorecard and the next hole.
- After hole 9: 10 s of the final card (lowest total wins), then a new course. Rotation: 1 course per turn.
- Joining mid-course: you start the current hole from the tee; holes you missed score 10.
- Bots fill up to 4 players. With the ball at rest they try shots in 36 directions at 6 strengths plus 8 putts at the
  cup by running them through `step_ball` (2 a tick, so no tick runs long), each as meant and a little off either
  way so they don't take shots that only work when hit perfectly, and score where each ends up by a distance field
  to the cup over the hole. They take 1-3 s over a shot and hit it off by up to 0.05 rad and 8% power (the difficulty
  knobs), which plays around par.

### Ghosts
- Every other ball is an entity, interpolated as usual, drawn as a see-through sphere in its player's color with a
  short fading trail and a name tag above it. Hidden within 1 m of the camera, so ghosts never block a shot, and in
  the cup.
- Your own ball is solid, in your color, with no tag.

### Look and sound
- Felt with a fine grain, rails as boxes, the cup a dark hole with a white rim tilted to the slope, a flag on a pole,
  windmills as a post with a cap and a paddle. Clouds below the holes, the next holes visible far off.
- Sounds: putter tock (louder for harder shots), rail clink, cup rattle, splash, a falling whistle into the void, a
  ripple of applause under par, a chime at the start of each hole, a soft wind.
- HUD: hole number, par and your strokes top left, the hole clock top center, the power bar while pulling, the feed
  top right, centered messages, a hint line, and the scorecard on Tab and between holes (players by hole, total,
  relative to par, ping, with a par row).

- Run: `cargo build -p minigolf --release --target wasm32-unknown-unknown`, then
  `cargo run --release -- server --game target/wasm32-unknown-unknown/release/minigolf.wasm`. Shipped in releases next to
  the others.

# fps16 — minimal CS 1.6 clone

Deadline: playable before 00:00. One weapon, one simple map.

## Run
- `cargo run --release -- server [port]` (headless, default 5000)
- `cargo run --release -- client <host:port> [--lag ms] [--jitter ms] [--interp ms]` (`--lag`/`--jitter` fake delay and bursty arrival to test netcode, `--interp` is the minimum remote-player buffer, default 50)

## Distribution
- `tools/release.sh` builds `dist/fps16-mac.zip`: a universal, unsigned `fps16.app` whose launcher self-updates from
  `$FPS16_PUBLIC/fps16.sha256` + `fps16.gz`, then asks to Join an address or Host a server on that Mac.
- `tools/release.sh publish` bumps `build_number.txt` and uploads to the VPS; the sha256 file goes last since it is the
  update trigger. The build number is compiled in; the server rejects clients on another build and they are told
  to restart (which updates them).

## Gameplay
- Players are capsules ("rounded cylinders"): radius 0.4, height 1.8, eye at 1.6.
- Movement: Quake/GoldSrc style (ground friction, accel, air accel, gravity, jump). WASD + mouse, space jumps.
- HUD: crosshair (gray while the gun cools down), HP, kills/deaths, net_graph line (ping is network round trip only, via
  a client timestamp the server echoes with its hold time) (ping, worst snapshot gap, correction, interp starvation).

## Weapons and economy (`src/weapons.rs`, CS 1.6 stats)
- Knife (always), Glock (T) / USP (CT) free, Deagle $650, MP5 $1500, AK-47 (T) $2500 / M4A1 (CT) $3100, AWP $4750
  (right-click scope), Kevlar+Helmet $1000. Buy with B then 1-5 during freeze + 20 s, within 15 m of your spawn.
- Slots: 1 primary, 2 pistol, 3 knife, Q last; R reload (auto when empty), draw delay on switch, semi-auto needs a
  click per shot. Mag + reserve ammo, refilled at round start.
- 100 HP. Damage by zone (head x4, legs x0.75), falls off with distance, armor absorbs per weapon penetration.
- Recoil kicks the view (bullets follow it, so pull down); spread grows with movement, air and recoil. Spread is
  seeded by the input seq so predicted and server bullets match. All weapon state lives in `PlayerState`.
- Money: $800 start, $16000 cap; kill $300 (AWP $100, MP5 $600, knife $1500); win $3250, loss $1400 + $500 per
  consecutive loss (up to 3); plant/defuse $300. Survivors keep guns; the dead restart with the team pistol.
- Each weapon has its own model (`src/model.rs`) and its own synthesized sound (silenced thwip for M4A1/USP, booming
  echoing Deagle/AWP with bolt work, harsh AK crack, light MP5/Glock), plus reload, buy, headshot, hit and hurt sounds.
- Movement speed depends on the weapon in hand: knife 108%, pistols/MP5 100%, M4A1 92%, AK 88%, AWP 84% (60% scoped).
- Player names: the logged-in user (`$USER`); bots are named "BOT Hans" etc.
- HUD: HP/armor/money, weapon + ammo, buy menu, kill feed, red flash when hit. Short tracers (AWP longer).

## Rounds (bomb defusal)
- Teams T and CT, humans auto-balanced on join, friendly fire off. Server-side bots fill both teams to 3v3 (or to the
  bigger human team); they path over the map grid via a random detour point each round (Ts share a random site, some lurk;
  CTs hold spots next to cover that watch the T entrance and move between them, facing the entrance), strafe in
  fights and only shoot once slowed down (counter-strafing), plant/defuse, call out spotted enemies so teammates rotate, and
  only notice enemies in a 120° field of view (callouts too), aim with a reaction delay, limited turn speed and angular aim error (~30% accuracy bot vs bot; `src/bot.rs`
  constants are the difficulty knobs).
- Map: dust, traced from a top-down overview image into an ASCII grid in `src/map.rs` (2 m cells, flat): B site
  top-left, A bottom-left, CT spawn between them, T spawn bottom-right, tunnels through the middle. Sites tinted red.
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

## Netcode (server authoritative, Valve-style)
- Plain UDP, postcard-serialized messages. Fixed tick 128 Hz on both sides; snapshots sent at 32 Hz, input packets at 64 Hz
  (CS 1.6 era rates: fewer packets suit Wi-Fi).
- **Client prediction**: each tick the client samples input, stamps a sequence number, simulates it locally with the
  same shared `step_player` fn the server uses, stores it, and sends the last few unacked inputs (redundancy vs loss).
- **Server**: queues each client's inputs and simulates one per tick (two when more than 2 are queued), so bursty
  uplinks don't make players teleport. Prints per-player loss / worst packet gap / buffer stats every 5 s. Sends a snapshot
  every tick with every player's state + the last input seq it processed for the receiving client.
- **Reconciliation**: on snapshot, client resets its own state to the server's and replays all inputs newer than the
  acked seq. Remaining visual error is smoothed out over a few frames.
- **Entity interpolation**: remote players are rendered in the past, interpolated between buffered snapshots. The delay adapts to the worst recent snapshot gap (min `--interp`, max 150 ms); if it still runs dry, players are extrapolated on velocity for up to 100 ms.
- **Lag compensation**: each input carries the server tick the client was viewing (interpolation time). On fire, the
  server rewinds other players to that tick (max 400 ms back) from its position history and raycasts there.

## Out of scope (for now)
Grenades, dropped/pickup weapons, halftime/match end, team switching, map height (ramps, stairs), anti-cheat, encryption, connection handshake/timeouts beyond the basics.

# PROGRESS (append-only)

## 2026-10-22 — iteration 1: bootstrap (PLAN 1.1–1.3)
- Installed Rust toolchain on the dev machine (Windows 11, MSVC host): rustup via winget, rustc 1.99.1 stable + rustfmt + clippy.
- Created Cargo workspace (edition 2024, resolver 3) with crates: sim, net (libs); server, client (bin `lgduel`), bot, tools (bins). Toolchain pinned in rust-toolchain.toml (1.99.1).
- `scripts/check.sh`: fmt --check, clippy -D warnings (all targets), release-dbg build, workspace tests, runs each binary. Future stages print an explicit `SKIP` naming the phase that adds them (no fake passes).
- Verified: `./scripts/check.sh` → CHECK GREEN (Windows, Git Bash). Tests: 2 passed (sim tick = 8000 µs at 125 Hz; protocol version nonzero).
- Measured: n/a (no runtime code yet).

## 2026-10-22 — iteration 2: PLAN 1.4 shared gameplay config
- Added `lgduel_sim::config::GameConfig` (physics, lg, rules sections; `DuelMode` vamp/classic, `HitMode` anti_lag/no_anti_lag) with serde + toml. Defaults = D3 table. Missing keys → defaults; unknown keys rejected; `validate()` enforces invariants (tick 10–1000 Hz, lifesteal ≤100, finite non-negative physics, hull sanity, refire > 0, spawn_health ≤ max_health, classic frag_limit > 0).
- Added `config/server.example.toml`; a test asserts it equals the defaults so docs cannot drift.
- Verified: `./scripts/check.sh` → CHECK GREEN (9s). sim tests: 8 passed (defaults valid, round-trip identical, empty doc = defaults, partial override, example file = defaults, unknown field rejected, 7 invalid docs rejected).

## 2026-10-22 — iteration 3: PLAN 2.1 math foundation
- `sim::math`: `Vec3` (Quake axes, no FMA: `mad` = two rounded ops), `Angle(u32)` = 2^32 units per turn (exact on the wire, wraps naturally, signed view for pitch + clamp), deterministic `sin_cos` (integer octant reduction + fixed Taylor polynomials in f64, only + and *), `angle_vectors` (Quake convention, roll 0), `StateHasher` (FNV-1a 64, stable forever; hashes exact f32 bits).
- Verified: check.sh GREEN. sim tests 15 passed. Max |sin/cos error| vs std over 100k angles < 1e-14 (asserted). Cardinal angles exact (sin 90° = 1, cos 90° = +0).

## 2026-10-22 — iteration 4: PLAN 2.2 fixed-tick loop
- `sim::tick`: `Tick(u32)` and `FixedStepper` — caller feeds elapsed µs; accumulator kept in µs·Hz units so non-divisor rates (60 Hz) never drift; catch-up capped (default rate/4 ticks) to avoid spiral after stalls; `alpha()` for render interpolation.
- Verified: check.sh GREEN (3s). Tests: exact 125 Hz stepping, 1 h of 1 ms frames at 60 Hz = exactly 216000 ticks, irregular frame sum exact, stall cap, alpha 0.5 at half tick.

## 2026-10-22 — iteration 5: PLAN 2.3 usercmd
- `sim::usercmd::UserCmd { tick, pitch, yaw, forward, right, buttons }`: exact `Angle(u32)` view, ±127 move axes, JUMP/CROUCH/FIRE bits (jump/crouch replace Quake's upmove). `from_keys` (opposite keys cancel), `sanitized()` for untrusted input (pitch clamp ±89°, no -128, unknown bits cleared).
- Verified: check.sh GREEN. Tests: key→axis mapping, sanitize of hostile values.

## 2026-10-22 — iteration 6: PLAN 2.4 world + map format
- `sim::world`: `Aabb` (strict overlap, Minkowski expand), `World { name, brushes, colors, spawns }`, TOML map loader with validation (≥1 brush, ≥2 spawns, min<max, finite, spawns not overlapping solids with the standing hull). Maps embedded via `world::builtin`.
- `maps/arena_flat.toml`: original CC0 2048×2048 open floor, 512+ walls to a ceiling at 1024, 4 spawns.
- Verified: check.sh GREEN. Tests: builtin map loads, bad maps rejected (spawn in floor, single spawn, inverted brush, parse error), AABB overlap/expand.

## 2026-10-22 — iteration 7: PLAN 2.5 trace
- `sim::trace`: swept AABB vs world brushes (6 half-spaces per expanded brush, Quake-style 0.125 surface epsilon), returns fraction/end/normal/start_solid/all_solid; `World::trace`, `World::trace_ray` (zero hull for hitscan).
- Verified: check.sh GREEN. Tests: fall onto floor stops at 24.125, wall hit normal + position, sliding epsilon above floor is free, start/all-solid flags, ray hit/miss, diagonal picks nearest plane.

## 2026-10-22 — iteration 8: PLAN 2.6 + 2.7 + 2.8 pmove (done together)
- These three items share one slide-move core and can't be tested apart without stubs, so they were built and verified together.
- `sim::pmove`: `PlayerMove` state (origin, velocity, ground normal, ducked, jump_held), `pmove()` per tick: duck check (stand only with room), ground probe 0.25u, walk move (friction w/ stop speed, cmd scale so diagonal = straight, ground-projected axes, crouch speed cap), air move (air accel), trapezoidal gravity, clip-plane slide move, step-up to step_height, velocity snap. Config gained view_height 26 / crouch_view_height 12, auto_hop, snap_velocity.
- Bug found by test and fixed: residual +1 u/s vertical velocity after landing (see DECISIONS D7).
- Verified: check.sh GREEN. sim tests 41 passed; pmove tests: stand still exactly, fall + land at z=24.125 with vz=0, ground accel → exactly 320, diagonal ≤ 321, friction stops 320→0 within 1 s, crouch walk = 80, can't stand under low ceiling, wall block + slide, step 16u yes / 24u no, jump re-press without auto_hop, auto hop keeps hopping, crouch eye 14u lower.

## 2026-10-22 — iteration 9: PLAN 2.9 + 2.10 golden movement & strafe-aim ratios
- `crates/sim/tests/movement_golden.rs`: standing jump apex/airtime, ground accel time, strafe jumping with an optimal-angle controller (speed never drops between hops), circle jump, and the four strafe-aim relative speeds measured from steady-state pmove velocities.
- Measured (pinned): jump apex 50.342u / 92 ticks; accel to 320 in 19 ticks; circle-jump takeoff 378.34 u/s; strafe-jump hops 494→643→729→826→905→998→1069→1142, 1145.74 u/s after 6 s; ratios mirroring 0.0000v, anti 2.0000v, HSW mirroring 0.2937v, HSW anti 1.7062v.
- Researched QL cvar defaults (3 web searches): no authoritative values found; KI-4 stays open.
- Verified: check.sh GREEN, 5 golden tests pass.

## 2026-10-22 — iteration 10: PLAN 2.11 + 2.12 combat
- `sim::game`: `Game` (config, world, tick, players, per-tick events), `Player` (pmove, view, health/armor, alive/respawn, weapon timer, lifesteal remainder, stats), `GameEvent` (Fire/Damage/Kill/Respawn), `hitscan()`, `Game::state_hash()`.
- Done together: hitscan alone has no observable effect without damage.
- Verified: check.sh GREEN; sim lib tests 51 passed. Game tests: 21 cells in 1.008 s with only 6/7-tick gaps; hit at 700u (100→93), miss at 900u; wall blocks; vamp 100% heals exactly dealt up to the cap; 33% lifesteal over 500 ticks earned == floor(dealt·33/100); 0% heals nothing; kill → dead untargetable → respawn exactly 125 ticks later at full health; knockback along beam; armor 30 dmg → 90 hp / 80 armor; hitscan nearest/occluded/behind.

## 2026-10-22 — iteration 11: PLAN 2.13 rules
- Fight lifecycle in `Game`: `fight_start` set when ≥2 players all alive; kill → `FightEnd{winner,loser,ticks}`; stats fights_won/fights_lost/fight_ticks; `frags` score; classic `MatchEnd` at frag_limit with score reset; vamp resets winner to spawn health (new `rules.reset_winner_health`, default true). State hash covers new fields.
- Verified: check.sh GREEN. New tests: vamp fight lifecycle (fight start at tick 0, FightEnd with ticks, no MatchEnd, winner back to 100, W/L + fight ticks on both players, next fight starts after respawn); classic frag limit 2 → exactly one MatchEnd and scores reset.

## 2026-10-22 — iteration 12: PLAN 2.14 determinism
- Added deterministic `atan2_det` + `look_angles` to `sim::math` (tests: < 1e-13 vs std over 100k points; look → angle_vectors round trip).
- `crates/sim/tests/determinism.rs`: seeded drivers (random A/D strafes 150–600 ms, 10% jump segments, range keeping 200–450u, aim with 1.5° vs 4° jitter, always firing) play 7500 ticks; inputs recorded, replayed on a fresh game; hashes equal + pinned. New check.sh stage prints the result.
- Measured: hash 0x9e74a8ea7918ee78; 994 damage events, 25 kills; accuracy a 57.1% / b 44.1% → 25W-0L in vamp (accuracy gap decides fights, as in the real format). 60 s sim runs in ~0.01 s (release-dbg).
- Phase 2 (sim core) complete. Verified: check.sh GREEN.

## 2026-10-22 — iteration 13: PLAN 3.1 client window
- `client`: winit 0.30 app + wgpu 30 `Gfx` (high-perf adapter, sRGB surface, present-mode picker, resize, robust acquire), `FrameStats` (nearest-rank p50/p99), arg parsing (`--frames`, `--vsync`, `--fps-cap`, `--version`). check.sh now runs `lgduel --version` (bare `lgduel` opens a window).
- Measured (Windows 11, NVIDIA RTX 4060, Vulkan, 1280×720, Immediate, clear-only frames): 1999 frames, p50 0.118 ms, p99 1.623 ms, max 2.051 ms, avg 5709 fps.
- Verified: `target/release-dbg/lgduel.exe --frames 2000` ran and exited cleanly; check.sh GREEN; unit tests: present-mode preference, percentiles, arg parsing.

## 2026-10-22 — iteration 14: PLAN 3.2 + 3.3 offscreen render + arena rendering
- `client::render`: flat fullbright pipeline (WGSL), box meshing with per-face shades, world mesh from map file, dynamic vertex buffer for per-frame geometry, Quake-axis camera + Hor+ FOV. `client::offscreen`: windowless device, timed frames, readback with row padding, PNG save, pixel checks. Windowed client now draws the arena from spawn 1 looking at spawn 2.
- Done together: an offscreen check of an empty frame proves nothing; the arena is what the check asserts.
- check.sh stage "client offscreen render" now live (replaces SKIP).
- Measured (RTX 4060, Vulkan, 1920×1080 offscreen, GPU wait each frame): 300 frames p50 0.098 ms, p99 0.659 ms, avg 7506 fps. Image: 4 distinct colours, floor 49.1% of pixels. Screenshot inspected visually: floor/walls/ceiling in correct perspective.
- Test bug caught and fixed: camera test had "right" as −x when facing +y; in Quake axes right is +x (left point now asserted negative too).
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 15: PLAN 3.4 + 3.5 mouse look + local movement
- `client::input`: `MouseSettings` (dpi, cm/360, invert), `MouseLook` (sub-count accumulation, pitch clamp), `Keys` → UserCmd. `client::local::LocalSession`: Game + FixedStepper, prev/cur origin interpolation, teleport snap. App: cursor grab (click to grab, Esc releases, focus loss clears keys), WASD/space/ctrl-C/mouse1, raw mouse motion, flags `--sens-cm --dpi --invert --synthetic-input`.
- Done together: mouse look has nothing to look at until the sim loop drives the camera.
- Verified: unit tests (full turn returns within 2 angle units after uneven fractional chunks; 10k×0.1 counts == 1000 counts; directions + clamp + invert; sens math; keys→cmd; 1000 fps vs 37 fps → same 250 ticks and same state hash; eye interpolation at half tick). Live run `lgduel --synthetic-input --frames 3000 --size 1920x1080`: 107 ticks in 0.86 s (125 Hz), yaw turned 228.6° = 3000×2 counts×0.0381°, ground speed 396 u/s; FRAMESTATS p50 0.204 ms, p99 2.096 ms, avg 3486 fps (RTX 4060, Immediate). check.sh GREEN.

## 2026-10-22 — iteration 16: PLAN 3.6 player models + forced enemy colours
- `client::players`: `EnemyColors` (head/upper/lower), `push_player` (three solid bands of the hitbox + facing nub, crouch aware). `LocalSession` now interpolates every player (teleport snap per player). Windowed client and headless render draw opponents.
- Headless check asserts forced enemy colours visible: head 120 px, upper 300 px, lower 340 px at 768u / FOV 110 / 1080p; screenshot inspected (three bands visible at far wall).
- Test fix: facing-nub assertion boundary (nub touches the hitbox face at x=15 → `>=`).
- Verified: check.sh GREEN; client tests 18 passed (incl. other-player interpolation at half tick).

## 2026-10-22 — iteration 17: PLAN 3.7a LG beam + sparks
- PLAN: split 3.7 into 3.7a (beam + sparks), 3.7b (hit sound); damage numbers moved to 3.8 (needs text).
- `client::effects`: BeamStyle none/thin/thick/zigzag, EffectSettings (beam colour, sparks on/off, spark size/colour), camera-facing segment quads, deterministic jitter, sparks reflected toward the shooter. Own beam predicted per frame; enemy beams from their state. Flags `--beam`, `--no-sparks`, `--spark-size`. `LocalSession::teleport`.
- Headless scene now: opponent at 400u, own beam firing. Measured (1080p): thin beam 3582–8066 px, zigzag 5207–5524 px, thick 10449 px; sparks 21–81 px at size 3, 130 px at size 6, 275 px at size 8; `--beam none` → 0 beam px; `--no-sparks` → 0 spark px.
- Iterated on spark visibility: first version lost half the streaks inside the target and the check at the 768u range limit was fragile; fixed by reflecting streaks and testing at 400u.
- Verified: check.sh GREEN (now includes none/no-sparks and zigzag/size runs). Screenshot inspected: beam from lower-right muzzle to opponent chest.

## 2026-10-22 — iteration 18: PLAN 3.7b hit sound
- `client::audio`: `synth_hit` (two-tone tick, 45 ms), pure `mix()` (overlapping voices, retire, clamp), `Audio` (cpal stream, f32/i16, silent fallback). `LocalSession.events` collects events of all ticks per advance; app plays a hit per own Damage event. Flags `--volume`, `--no-sound`, `--sound-test`.
- Tooling: edit helper now matches ignoring whitespace/commas (rustfmt reflow broke two scripted edits).
- Verified: `lgduel --sound-test` → "AUDIO Speakers (Realtek(R) Audio) via S/PDIF @ 48000 Hz, 2 ch … played 5 hit sounds" (audibility needs a human, KI-9). Tests: sound shape (2160 samples @48k, peak 0.9, decays >20×, starts at 0), mixer overlap/retire, silent handle, 100 ms firing at enemy → exactly 2 own Damage events. check.sh GREEN.

## 2026-10-22 — iteration 19: PLAN 3.8 HUD
- `client::font` (original 5×7 glyphs: 0-9, A-Z, punctuation), `client::hud` (CrosshairStyle, HudSettings, HudInfo, DamageNumbers with merge window, NDC quads, projection, build), HUD pipeline + buffer in `Renderer`, HUD in windowed and headless modes.
- Headless now simulates firing (virtual 2 ms/frame) → "SIM hits 12" (84 dmg, opponent 16 hp, LG 100%). Measured pixels (1080p): crosshair 68, HUD text 7118, damage numbers 279, beam 8064, sparks 68.
- Visual review found and fixed: damage numbers stacking into an orange smear (now merged total "84"), poor text contrast on bright walls (drop shadow).
- Verified: check.sh GREEN; new tests: glyph table well-formed/unique, fallback box, NDC mapping, crosshair parts/off, text width, damage merge/expiry, projection centre/behind.

## 2026-10-22 — iteration 20: PLAN 3.9 bot crate
- `crates/bot/src/lib.rs`: `Bot::think(&Game, me) -> UserCmd`, `BotConfig`, `StrafePattern`, `AimSkill` presets, deterministic `Rng`.
- Measured: accuracy vs random A/D strafer (20 s): aimbot 100.0%, hard 66.0%, easy 13.5%; vamp 60 s hard (68.6%) beats easy (11.2%) 26W–0W; aimbot vs still target hits every cell; pattern relative speeds mirror 0.11 / anti 1.86 / HSW 0.31 / HSW anti 1.74 v.
- Test-measurement bugs found and fixed: lateral speed must be measured against the live line between players (anti-mirroring orbits; world-y measure gave 0.84v), and HSW needs a range band (walking through the opponent flipped the line).
- Verified: check.sh GREEN; bot tests 7 passed.

## 2026-10-22 — iteration 21: PLAN 3.10 offline practice vs bot
- Client depends on `lgduel_bot`; default opponent is a medium random-A/D bot. Flags `--bot still|ad|mirror|anti|hsw|hswanti`, `--bot-skill easy|medium|hard|aimbot`, `--no-bot` (idle dummy). Exit prints BOT stats.
- Live windowed run (`--synthetic-input --frames 20000 --no-sound --bot-skill hard`): 583 ticks, bot 50.8% acc, 200 dmg, 2 frags vs the scripted player; 4286 fps avg, p99 1.685 ms (1280×720, RTX 4060).
- New test `offline_practice_vs_bot_plays_out`: 20 s at uneven ~240 fps → exactly 2500 ticks; bot 269 shots / 105 hits / 7 frags vs a strafing non-shooting player. First threshold (300 shots) was my arithmetic mistake (bot doesn't fire during the 7×1 s respawns); corrected to > 200.
- Phase 3 (local playable) complete. Verified: check.sh GREEN.

## 2026-10-22 — iteration 22: PLAN 4.1 transport + channels
- `net::wire` (LE codec, varint/zig-zag, panic-free reader), `net::transport` (Transport trait, UdpTransport, MemNet with latency/jitter/loss), `net::conn` (seq/ack bits, reliable ordered resend, unreliable payload, RTT/jitter/loss stats).
- Bug found by test and fixed: loss was only counted when a history slot was reused (1024 packets later), so short sessions always reported 0% loss; now counted when a packet leaves the 33-packet ack window.
- Verified: net tests 13 passed — 500 reliable messages delivered exactly once in order over 40±20 ms / 20% loss; RTT estimate 112 ms for 2×50 ms (+step quantisation); 10% loss link reports loss within 5–15%; duplicates/garbage rejected; MTU cap; real UDP loopback; MemNet loss rate 5% ±1% and deterministic; jitter reorders. check.sh GREEN.

## 2026-10-22 — iteration 23: PLAN 4.2 protocol messages
- `net::protocol`: Connectionless handshake messages + cookie, `Inputs` (8× redundant cmds), `InputBuffer` (dedup, sanitize, late/dup counters, lead), `Control`. `net::snapshot`: `Snapshot` capture/apply/encode/decode with all player state and events.
- Verified: net tests 24 passed — handshake round trips, no-amplification sizes, name truncation, cookie keyed on secret/addr/nonce, inputs exact, input buffer survives a lost packet via redundancy (10 received, 6 dups), sanitisation, snapshot of a 900-tick fight → decode → apply → identical state hash and identical next tick; truncated snapshots rejected. Full snapshot 173 bytes. check.sh GREEN.

## 2026-10-22 — iteration 24: PLAN 4.3 snapshot delta compression
- `Snapshot::encode_delta/decode_delta/delta_base`, field masks, `SnapshotHistory`.
- Tooling: bash heredoc containing Python triple quotes failed to parse in the Bash tool; switched that edit to the Edit tool (no changes had been applied).
- Measured: 1500-tick fight, every 5th snapshot lost → every delivered delta reproduced the exact snapshot; 118,056 B vs 285,535 B full (41%); unchanged state 25 B; wrong/missing baseline rejected.
- Verified: net tests 27 passed; check.sh GREEN.

## 2026-10-22 — iteration 25: PLAN 4.4 server binary
- `crates/server/src/lib.rs`: `ServerConfig` ([server] + gameplay), `Server<T: Transport>` (poll/tick, challenge handshake, accept with config TOML, reject wrong version/full, ignore bad cookies, InputBuffer per client, last-real-command repeat, delta snapshots per client, timeout/disconnect, operator log, client stats). `main.rs`: CLI (`--config --port --bind --duration-secs --version`), real-time 125 Hz loop.
- Bug found by test and fixed: startup ticks before a client's first command were counted as "missing" because the fallback default command was stored as last_cmd.
- Verified: server tests 5 passed (config sections + example file; MemNet handshake → 2 s of inputs → snapshots show 320 u/s walking, 0 missing, RTT > 15 ms; spoofed cookie ignored, wrong version rejected; timeout removes player; 5% loss + jitter with 8× redundancy → 0 missing commands over 10 s, loss estimate 3.4%). Real binary: 2.0 s → exactly 250 ticks, clean exit. check.sh GREEN (now also runs the server for 0.5 s).

## 2026-10-22 — iteration 26: PLAN 4.5 clock sync + tick alignment
- `crates/net/src/client.rs`: `NetClient<T>` (connect/accept/reject, delta snapshot reception with baselines, arrival-jitter estimate, server-tick estimate, target lead, dilated client clock, `advance()` → ticks due, `send_cmd` with 8× redundancy + snapshot ack, `disconnect`).
- `crates/server/tests/clock_sync.rs` (NetClient ↔ Server over MemNet): 50±10 ms → 0 late commands in 10 s, max server buffer 6 ticks, RTT 101.7 ms, lead 8.8 ticks; latency → 150±10 ms → 0 late in 10 s after 3 s, RTT 301.1 ms, lead 21.6; 20 s: client 2500 ticks = server 2500; full server → Rejected("server full").
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 27: PLAN 4.6 prediction + reconciliation (also satisfies 4.11)
- `crates/net/src/predict.rs`: Predictor (predict per cmd, reconcile on snapshot, error stats, decaying visual offset, eye/render origin).
- `crates/server/tests/prediction.rs` (runs in check.sh → PLAN 4.11): NetClient + Predictor vs Server over MemNet with a busy script (strafes, jumps, crouch, turning) for 20 s.
- Bug found by test and fixed: before the first ack, RTT was 0 → lead ≈ 1 tick → first commands late → one-off 271u misprediction (50 ms) and 20 missing commands (5% loss run). Fix: RTT seeded from the handshake.
- Measured: max prediction error 0.0000u (0 mispredicted ticks) at 0 ms (2498 checked), 50 ms (2461), 100±20 ms (1376), 50±10 ms + 5% loss (1814); 0 missing commands in all runs. Clock sync still: lead 9.6 ticks @50±10, 21.3 @150±10.
- Test bound changed: "checked > 1500" → "> 1000" (with ±20 ms jitter, out-of-order snapshots aren't reconciled/checked; error itself is 0).
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 28: PLAN 4.7 entity interpolation
- `crates/net/src/interp.rs`: Interpolator (bracketing blend, short-way angle lerp, teleport guard, starvation stats, optional capped extrapolation, adaptive delay with asymmetric rate + decaying starvation floor). NetClient: `snapshot_clock` (fastest-path arrival clock, smoothed), `arrival_spread_ticks`, `recommended_interp_delay`.
- `crates/server/tests/interpolation.rs`: client A renders client B walking (12 s, 1 ms frames).
- Iterations driven by measurement: (1) render clock = estimated server time → 100% starved (design error); (2) fixed 3-tick delay at 50±10 ms → 0.76% starved; (3) adaptive delay; (4) delay creeping up too slowly → asymmetric growth; (5) per-frame target overwrote the starvation bump → separate decaying floor. Snapshot-clock smoothing kept (sound, though not the cause).
- Measured: 50 ms → delay 1.5, 0.00% starved, max step 0.336u (= 320 u/s + 5% dilation); 50±10 → delay 3.88, 0.00%, 0.336u; 100±20 ms + 2% loss → delay 6.25, 0.03% starved (one 3 ms hold), max step 1.04u. Thresholds per condition recorded in DECISIONS D26.
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 29: PLAN 4.8 lag compensation
- sim: `Game::step_with(cmds, rewind)` hook (step = no rewind). net: protocol v2 with per-command view time; `InputBuffer::push_with_view/take_with_view`; `NetClient::send_cmd_with_view`. server: rewind buffer, interpolated rewound hitboxes, clamp to max_rewind_ms, `RewindStats`.
- `crates/server/tests/lagcomp.rs`: shooter at 100 ms RTT aims at what its interpolation shows; strafer A/D.
- Measured: anti-lag 100.0% (219 shots, mean rewind 119 ms) vs no-anti-lag 24.7% at 100 ms RTT; anti-lag 100.0% at 100±20 ms (mean rewind 143 ms); 600 ms RTT → 214/214 rewinds clamped to 200 ms, accuracy 12.6%.
- Verified: determinism golden hash unchanged; check.sh GREEN.

## 2026-10-22 — iteration 30: PLAN 4.9 bot as network client
- `crates/bot/src/netbot.rs` (NetBot), `crates/bot/src/main.rs` (UDP bot binary with BOTRESULT output), Predictor `external` corrections, server dev-dep on bot, `crates/server/tests/netbots.rs`.
- Finding: the victim bot showed mean 1.08u / max 896u "prediction error" — caused by being hit/killed/respawned (unpredictable for the victim), not desync. Metric now excludes those (counted as external) → both bots 0.0000.
- Finding → PLAN 5.6 + KI-10: events ride only in the snapshot carrying them; lost snapshot = lost hit sounds/kill notices.
- MemNet duel (25±5 ms, 1% loss, 30 s): hard 9W–0L, 42.1% vs 9.3% accuracy; lifesteal_earned == damage_dealt for both; prediction 0.0000 over 3493 checks.
- Live UDP (server + 2 bot binaries, 10 s): both connected/left cleanly; hard 62.6% acc 4 kills, easy 10.7%; lifesteal exact; prediction 0.0000 (1235/1173 checks); server 1499 ticks in 12.0 s.
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 31: PLAN 4.10 headless match in check.sh
- Server: `--stats-file` (TOML, includes departed players), `Server::stats_toml`. Tools: `netem` UDP proxy (per-client upstream sockets, delay/jitter/loss both ways, profiles), `matchrun` orchestrator, `lgduel-tools match|netem`. check.sh stage "headless match" live (was SKIP).
- Measured (60 s each, parallel, check.sh run): clean — hard 58.7% 21 kills vs medium 18.1%, earned==dealt (3045/945); lossy — 38.1% vs 38.6%, netem dropped ~2%, earned==dealt (2991/3030), prediction mean 0.0038 / max 6.69u (hard) and 0.0000 (medium); highping (RTT ~205 ms) — 9.2% vs 11.8%, earned==dealt, prediction 0.0000. check.sh total 71 s (was ~11 s).
- First non-zero misprediction seen (lossy, one bot, max 6.7u): consistent with an occasional command arriving after its tick under jitter (server repeats last command). Within threshold; to be characterised in PLAN 5.2.
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 32: PLAN 4.12 real client connects
- Refactor: `client::scene` (FrameView, scene_mesh, track_hits, hud_mesh) shared by offline (`LocalSession::frame_view`) and online (`client::net_session::NetSession`). Offline headless output identical before/after (same pixel counts).
- `lgduel --connect`, `--name`, `--secs`; `run_headless_online`; `lgduel-tools client-smoke`; check.sh stage.
- Bug caught by running it: `--headless --connect` hit the offline headless branch first (only the bot connected); branch order fixed.
- Measured: headless online — connected, opponent visible 2102/2105 frames, RTT 3.9 ms, adaptive interp delay 1.85 ticks, prediction 575 checks mean 0.0000, remote opponent pixels head/upper/lower 224/526/…; screenshot inspected (bot rendered, camera tracking). Windowed online (`--synthetic-input --frames 8000`): connected, took damage from the bot, 4079 fps avg, p99 1.513 ms, clean disconnect.
- Phase 4 (networking) complete. check.sh GREEN (80 s).

## 2026-10-22 — iteration 33: PLAN 5.1 netem layer (already satisfied; bookkeeping)
- In-process: `lgduel_net::transport::MemNet` — deterministic µs clock, latency, ±jitter (reorders), loss; tests memnet_latency_and_order, memnet_loss_rate_and_determinism (5% ±1%), memnet_jitter_reorders.
- Real sockets: `lgduel-tools netem` UDP proxy (per-client upstream, both directions, profiles clean/lossy/highping); test proxies_with_delay; used by the headless match (lossy run dropped ~2% as configured).
- No code change; verified by the existing tests in check.sh (GREEN at e5260b6).

## 2026-10-22 — iteration 34: PLAN 5.2 netem matrix
- `crates/server/tests/netem_matrix.rs`: 16 cells, per-cell metrics table (RTT, prediction mean/max/checks, missing %, interp delay, starved %, max jump), thresholds asserted (D31); check.sh "netem matrix" stage live (was SKIP), prints the table; runs in ~0.3 s.
- Product fix found by the matrix: `NetClient` arrival-spread now counts every snapshot arrival, not only those arriving as newest → interpolation test improved to 0.00% starved / max 0.320u in all 3 conditions (was 0.03% / 1.04u in the 100±20 ms + 2% case).
- Test bug found and fixed: the matrix only called `interp.sample` after warm-up, so the adaptive delay (which adapts inside `sample`) hadn't converged on the 200 ms rows → spurious 3.5u jumps right after warm-up.
- Results table recorded in DECISIONS D31. Verified: check.sh GREEN.

## 2026-10-22 — iteration 35: PLAN 5.3 hit-reg agreement
- `crates/net/src/hitreg.rs` (HitReg, HitRegStats), `crates/server/tests/hitreg.rs`, NetSession records verdicts per fired command (others sampled before command generation so the verdict uses exactly what is drawn); headless online run now fires and prints agreement.
- Measured: anti-lag 100.0% (50 ms, 100 ms RTT) vs no-anti-lag 54.0% / 38.2%; 150±40 ms + 2% loss: 77.4% at cap 200 → 100.0% at cap 300 (no-anti-lag 37.0%). Real UDP smoke: 100.0% of 64 shots.
- Test design fixes: ±1.5° aim noise never produced on-screen misses (false-miss path untested) → ±4°; shot-count bound 150 → 100 for lossy links (events lost with snapshots, KI-10).
- Finding: rewind ≈ RTT + lead margin + interp delay → default 200 ms cap covers RTT ≲ 140 ms (D32, KI-11).
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 36: PLAN 5.4 rubber-band detector
- Predictor: `max_correction`, `rubber_bands` (> 1u, external corrections excluded; dead counts as external). Matrix table gained own-correction columns + assertion (≤ 2 per cell). BOTRESULT reports them; `lgduel-tools match` asserts ≤ 1 per 10 s per bot in all three profiles (lossy + highping required by the plan were already in check.sh).
- Measured: matrix 0 rubber-bands in 15/16 cells, 2 (3.33u max) at 0 ms + 5% loss; headless 60 s matches: see check log (all within threshold).
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 37: PLAN 5.5 net-graph
- Connection byte counters; `hud::NetGraph` + overlay; NetSession computes it each frame; F3 / `--no-net-graph`; headless online renders the HUD and asserts net-graph values + pixels; client-smoke prints NETGRAPH.
- Measured (real UDP smoke): ping 3.9 ms ±1.9, loss 0.00%, tick 125 Hz, snapshots 125/s, interp 16.3 ms, in 15.5 KB/s, out 20.5 KB/s, rubber-bands 0, hitreg 100%; net-graph 4424 px. Screenshot inspected.
- Tooling: bash heredocs with long embedded Python failed to parse again; edit scripts now written with the Write tool.
- Priority change (user, 2026-10-22): friend-test first — next a release package + "play with a friend" guide, then continue the plan.
- Verified: check.sh GREEN.

## 2026-10-22 — iteration 38: friend-test release (F.1, PLAN 7.1) + stall robustness
- `crates/server/src/host.rs` shared by `lgduel-server` and new `lgduel host`; `.cargo/config.toml` static CRT; `PLAY.md`; `scripts/package.sh`; `/dist` ignored. PLAN gained F.2 (GitHub publish, user request).
- Packaged: `dist/LGDuel-windows-x64.zip` 4.26 MB, packaged smoke OK (ping 4.2 ms, 125 snapshots/s, hitreg 100%), exes import no VC runtime DLLs.
- A full check after the rebuild failed (lossy 15 / highping 8 rubber-bands). Investigation: (1) CPU-saturation repro found a real bug — clock snap re-issued ticks → fixed + regression tests; (2) late-command feedback loop added (+test); (3) harness stopped netem before in-flight Disconnects → fixed; (4) remaining failures were startup stalls of freshly built processes → bots measure after 5 s warm-up (threshold scaled accordingly).
- Verified: two consecutive full checks from a fresh build GREEN (84 s, 77 s), all bots 0 rubber-bands / 0 late commands after warm-up; 60 s match under 100% CPU load passes.
- Not verified: a run on a second PC / by a second human.

## 2026-10-22 — iteration 39: F.2 publish to GitHub (user request)
- README.md (features, 3-step play-with-a-friend, offline practice, controls, hosting, build from source, layout, license), MIT LICENSE; package includes them.
- gh 2.103.0 installed (winget, user-approved); user logged in as s0pk. History rewritten to the noreply email (39 commits, verified no other address remains), repo `s0pk/lgduel` created public and pushed; tag v0.1.0; release "LGDuel v0.1.0 — first playable test build" with `LGDuel-windows-x64.zip` (4,259,676 bytes, built from ec34411).
- Verified: release asset downloaded from GitHub into a fresh folder, unzipped, VERSION ec34411; packaged `lgduel host` + bot + headless client: both connected, ONLINE CHECK OK (ping 3.9 ms, 125 snapshots/s, hitreg 100%).
- Note: Windows refuses to start executables from very long working-directory paths (scratchpad) — use short paths for such smoke tests.

## 2026-10-22 — iteration 40: M1.1 egui integration + pause menu
- v0.2 plan approved by the user; PLAN gained M1–M5 items (D38). egui 0.37 verified to resolve to exactly wgpu 30.0.1 / winit 0.30.13.
- `client/src/ui.rs` (Ui: context, egui-wgpu renderer, egui-winit state, headless runs, paint, pending textures), `client/src/menu.rs` (Pause / Settings placeholder; Resume / Disconnect-or-Leave / Quit), `client/src/ttf.rs` (in-memory TrueType from our pixel glyphs), font.rs gained 13 punctuation glyphs + `chars()`/`glyph_rows()`; Esc toggles the menu; `--headless --menu` check in check.sh.
- Debugged by measurement: (1) nothing drawn → egui shapes 13 but 0 texture uploads in the painted frame (font atlas came in an unpainted warm-up frame) → keep deltas; (2) tofu boxes → no font (egui default fonts are a feature with non-MIT licences) → generated font; (3) menu check thresholds set after first real measurement (panel 10,211 px; threshold 5,000); centre-screen checks skipped in menu mode.
- Verified: menu screenshot inspected (pixel-font title + 4 buttons); font tests 2 passed (skrifa parse, mapping, metrics, distinct glyphs); windowed smoke 2000 frames 5200 fps p99 0.457 ms; check.sh GREEN (87 s).

## 2026-10-22 — iteration 41: M1.2 settings file
- `client/src/settings.rs`; serde on MouseSettings/EffectSettings/BeamStyle/HudSettings/CrosshairStyle/EnemyColors; Options = Settings + per-run fields (regex-migrated all opts.* accesses); `--settings`, `--no-settings`; practice bot from settings.
- Verified: unit tests (round trip, partial/unknown keys, sanitize, save/load/broken file) — client 42 passed; live: first windowed run created the file; hand-edited cm_per_360 = 55 → "MOUSE 55 cm/360 … 0.02078 deg/count"; `--sens-cm 20` overrides for the run without changing the file; compact TOML output inspected; check.sh GREEN.

## 2026-10-22 — iteration 42: M1.3 settings menu
- `menu.rs` rewritten: `Menu {screen, tab, dirty}`; Settings window (left side so the live crosshair/HUD/beam act as the preview) with tabs Mouse (cm/360, DPI, invert, deg/count), Video (FOV, fullscreen toggle, window size, vsync, FPS cap), Visuals (beam style/colour, sparks + size, enemy head/body/legs colours, crosshair length/gap/thickness/dot/colour, damage numbers, net-graph), Audio (volume, test sound), Player (name, practice bot on/pattern/skill).
- App: duplicate hud/colour copies removed (reads `opts.s`), `apply_settings` (mouse, audio incl. reopening when unmuted, vsync via new `Gfx::set_vsync`, window size), `save_settings` on Back / Esc-close when dirty. `--menu pause|settings` for headless; check.sh renders both.
- Verified: settings screenshot inspected (Visuals tab); headless menu checks OK (pause 10k px, settings 202k px panel); check.sh GREEN. Interactive clicking not automated — needs a human pass (noted for the release smoke).

## 2026-10-22 — iteration 43: M1.4 fullscreen
- Window created borderless-fullscreen when `video.fullscreen`; F11 and Alt+Enter toggle (saved immediately); Video-tab checkbox applies live; `--fullscreen` / `--windowed`.
- Verified: `lgduel --no-settings --fullscreen --synthetic-input --frames 1500` → surface 1920×1080 (native, window size setting 1280×720), 4377 fps avg, p99 0.595 ms; check.sh GREEN. M1 (UI + settings + fullscreen) complete.

## 2026-10-22 — iteration 44: M2.1 info query
- protocol: `InfoRequest{nonce}` padded to 128 B, `InfoResponse{nonce, ServerInfo{protocol, name≤32, map≤32, players, max, classic, lifesteal %, anti_lag, tick_rate}}` (≤ request size). Server answers from `Server::info()` regardless of protocol version (browser shows incompatibility). `net::browser::Pinger` (multi-server queries, RTT, timeout).
- Verified: protocol test (round trip, truncation, response ≤ request); `tests/browser.rs` over MemNet 40 ms → info fields correct, RTT 80–82 ms, no answer from an empty address, timed-out query dropped; check.sh GREEN.

## 2026-10-22 — iteration 45: M2.2 main menu state machine
- menu.rs: screens Main / Practice / Online / Connecting (+ Pause, Settings with return target), actions StartPractice / Connect / CancelConnect, Esc-back; main.rs: `session: Option<Session>`, start_practice / start_online / check_connection / leave_session / end_session, orbit camera, `--practice`, default port in `resolve`; NetSession `server_silent`.
- Verified (real windowed runs): no args → main menu (3746 fps); `--synthetic-input` → practice; `--connect` to `lgduel host` → "ONLINE joined", net-graph live; connect to a dead port → "MENU No answer from 127.0.0.1:27999 …"; host shutting down mid-game → "MENU Lost connection to the server"; main-menu screenshot inspected; check.sh GREEN (now also renders the main menu).

## 2026-10-22 — iteration 46: M2.3 server browser
- `config/servers.toml`, `client/src/browser.rs` (Browser: background lookups, Pinger, rows/status), Online screen table + Refresh + Add/Remove + direct connect; per-screen fixed window widths.
- Bug found by test: inline DNS resolution in refresh blocked (and inflated pings) → background threads.
- Verified: browser tests (shipped list parses; real UDP server answers with max 16 and RTT < 100 ms; unresolvable name → bad address); Online screen rendered against a live `lgduel host` ("LGDuel Server 6 ms 0/16 arena_flat Vamp 100% On Join") and inspected; check.sh GREEN (renders main/online/pause/settings screens).

## 2026-10-22 — iteration 47: M2.4 in-app host
- server host.rs: `config_from_args`, stoppable `run_config` with bind callback; client: `HostSettings`, Host screen, `HostHandle`, `start_host`, hosting info in pause menu, `--host-game`, `--menu host`.
- Verified (real runs): `--host-game` → "HOST Hosting: friends connect to 192.168.1.45:27975", self-joined; external `lgduel-bot` joined as player 1 and fought (2 kills, 200 dmg); port already in use → "MENU Can't host: can't use port 27976: … (os error 10048)"; Host screen screenshot inspected; check.sh GREEN. M2 (launcher) complete.
- Test-harness slip: a trailing `&` backgrounded my whole `&&` chain (variables unset) — first run invalid, rerun with separate lines.

## 2026-10-22 — iteration 48: M3.1 convex brush collision
- `crates/sim/src/brush.rs` (ConvexBrush, Plane, Rise, ramp/from_planes/vertices/face/contains + tests), `trace_convex` + merged `World::trace`, `World.convex`; `crates/sim/tests/ramps.rs`.
- Verified: brush tests (ramp has 6 vertices, 5 faces + 2 bevels, face polygons 4/4/4/3/3, all four rise directions, unbounded/empty rejected); ramp tests 6 passed (box rests on 30° slope within 0.3u of analytic height with slope normal; walks up onto platform at top height; walks back down to the floor; cannot climb 55° (max < half height); no sinking at seam positions −14…30; jump off the top clears platform + 30u); determinism golden hash unchanged; check.sh GREEN.
- Test fixes (my setup errors): platform too short (player walked off its far end); jump timed by tick instead of position.

## 2026-10-22 — iteration 49: M3.2 map format + convex rendering
- world.rs: `[[ramp]]` (min/max/rise/color → wedge), `[[stairs]]` (steps 1–64 → boxes climbing toward `rise`), `[[convex]]` (raw planes); errors name the entry; spawns rejected if inside a convex solid. render.rs: `push_convex` (fan-triangulated faces from `ConvexBrush::face`), `shade_for` (normal-blended fullbright shade, identical to box shades on axis normals).
- Verified: map test (floor + 4 stairs + ramp + convex load; tallest step at the −x end reaching max.z; bad rise / steps 0 / spawn inside ramp rejected with reasons); render test (ramp → 8 triangles, shades); client 45 / sim 59 tests; check.sh GREEN.


## 2026-10-22 — iteration 50: M3.3 new maps + map agreement
- maps/arena_pillars.toml (4 pillars, raised centre with ramps E/W and stairs N/S), maps/arena_tiers.toml (ground, mid deck 96u via ramp + stairs, high deck 192u via ramp, low cover wall); both original (CC0).
- sim: `World::content_hash` (parsed geometry, not colours/bytes), `builtin::{ALL, source, load, matching}`.
- net: protocol v3, `Accept{.., map, map_hash}`, `NetClient.map`. server: sends name + hash; `--map`; example config gains `[server]` (still = defaults). client: NetSession/NetBot switch to the announced built-in map or disconnect with `map_error` (menu shows "Can't join: …"); renderer re-uploads world on hash change; Practice + Host screens get a Map combo (settings `practice.map`, `host.map`, sanitised); `--map`.
- tools: `match --maps a,b,c` (per profile); check.sh runs clean@arena_flat, lossy@arena_pillars, highping@arena_tiers and renders both new maps.
- Verified: tests — builtin maps load (names match, distinct hashes), hash ignores colour/CRLF but tracks geometry/spawns, server tests/maps.rs (bots holding arena_flat switch to pillars/tiers and fight, prediction exact; edited same-name map → "differs" + disconnected; unknown name → "doesn't have"), settings sanitise unknown map; headless screenshots of both maps inspected (ramps, stairs, decks visible; RENDER CHECK OK); check.sh GREEN 101 s (matches on all three maps OK).


## 2026-10-22 — iteration 51: M3.4 map movement goldens
- sim/tests/map_movement.rs: every spawn on every built-in map settles without sliding; arena_tiers ground → ramp → mid deck → south → ramp → high deck (golden ticks 224 / 677, ramp speed 300 = 320·cos 20.6°); six 16u stairs keep 320 u/s (golden top tick 128); run off the high deck lands on the mid deck and keeps running; arena_pillars over the centre platform (walks on top >100 ticks, ramp-top hop 14.6u — Q3 kickoff, D46).
- Fix: sim `example_config_matches_defaults` broke in iteration 50 (example file gained `[server]`, which GameConfig rejects); the test now drops `[server]` before parsing. Iteration 50's check run predated that edit — I only ran the server lib tests after it.
- Headless matches + screenshots on every map were added in iteration 50 (check.sh).
- Verified: map_movement 5 passed; check.sh GREEN 102 s (matches clean@arena_flat, lossy@arena_pillars, highping@arena_tiers OK).


## 2026-10-22 — iteration 52: M4.1 multi-arena server
- server: `Arena { game, rewind }`, `Server.arenas` + `game()` accessor, `max_arenas` setting (validated > 0, in example config), server-wide player ids, `place()` auto-queue/pairing, `move_client` (stats carried in `past`), per-arena `Arena::step` (anti-lag unchanged), snapshots per client from its arena (queued → arena 0), `ClientInfo.arena`. sim: `Game::add_player_with_id`, `PlayerStats: AddAssign`.
- client/bot: when not in the snapshot (queued), send idle commands instead of nothing (the server timed out a queued bot before this fix).
- Verified: server tests/arenas.rs — 6 bots in 4 arenas → pairs in arenas 0/1/2, arena 3 empty, ticks equal, damage only within pairs (dealt = partner's taken), each bot's snapshot has exactly its pair; 5 bots / 2 arenas → bot 4 queued, stays connected, takes the place when a player leaves arena 0 and fights; 4 bots / 3 arenas, one leaves each pair → the lone players are paired into arena 0 and keep fighting, stats file includes the moved player; all earlier server tests pass unchanged (tests now use `game()` / `arenas[0].game`); check.sh GREEN 102 s.


## 2026-10-22 — iteration 53: M4.2 practice bot, spectate, status
- net: protocol v4, `ArenaStatus`, `Control::{Status, Spectate}` (+ round-trip tests), `NetClient.status`, `set_spectate`.
- server: depends on lgduel-bot; `practice_bot` / `practice_bot_after_ms`; `Arena.bot`, `lone_ticks`; `free_id` (clients + bots); bots think on the real arena game each tick; `set_spectate`; `watched_arena` (busiest); status computed and sent on change.
- client: `FrameView.{spectate, status}`, `SpectateView`; NetSession spectator view (follow first living player, their angles/beam/HUD numbers, net-graph), `status_text`, `set_spectate`; HUD `status_line`; Esc menu Spectate/Play (online); `--spectate` for headless online runs. tools: `client-smoke` runs play + spectate in parallel and checks the status lines.
- Verified: arenas tests 5 passed (new: lone player gets the bot only after 2 s, fights it, bot leaves when a 2nd human joins and status names the opponents; queue #1 watching arena 0 with the duellists in its snapshots, spectating frees the slot for the queued player, Play re-queues); real runs: lone headless client vs practice bot ("ARENA 1 - PRACTICE BOT…", 80% acc, server log shows bot join/leave), headless spectator of alice vs bob (screenshot inspected: first-person from a bot, both beams, status + net-graph); check.sh GREEN 99 s incl. both client smokes.


## 2026-10-22 — iteration 54: M4.3 reliable events (PLAN 5.6, KI-10)
- net: `Snapshot.{reliable_events, event_seq}`, `is_reliable_event` (all but Fire), shared `write_events`/`read_events` (count, seq only if non-empty); NetClient `take_reliable_events` (wrap-safe, delivers each seq once into `events`, counts `event_gaps`), `reliable_events_delivered`; protocol v5.
- server: per-client reliable queue + `event_base` + `event_marks` (snapshot tick → end seq), trimmed on ack (`Client::ack_events`), cap 256 (`events_dropped`); `ClientInfo.{events_sequenced, events_dropped}`.
- Verified: snapshot tests (reliable events round-trip in full and delta encodings; unchanged-state delta now 26 bytes, +1); server tests/events.rs at 25% loss (measured 21–23%), 40±10 ms, 30 s, short fights: both clients got 184/184 events, 0 gaps, 0 dropped, frags happened; all net/server tests; check.sh GREEN 99 s. PLAN 5.6 ticked, KI-10 resolved.

## 2026-10-22 — iteration 55: M5.1/M5.2/M5.3 prep (blocked on Linux), plan reconciliation
- Written: .github/workflows/ci.yml (Linux check.sh with Mesa lavapipe + ALSA headers; Windows fmt/clippy/tests/build; `deploy` job: install.sh on the runner's systemd with a fresh tarball + a bot joining, re-run keeps config, Docker image build + bot joins), release.yml (tag v* → Windows zip + Linux server tarball → GitHub release with RELEASE_NOTES.md); ci/check.Dockerfile + scripts/linux-check.sh; check.sh/package.sh honour CARGO_TARGET_DIR. Dockerfile, .dockerignore, docker-compose.yml, deploy/lgduel-server.service (hardened), deploy/install.sh (latest release or LGDUEL_TARBALL; keeps config; ufw), DEPLOY.md. README/PLAY rewritten for menus, servers, maps; RELEASE_NOTES.md (v0.2.0).
- Pushed branch v0.2-dev (main untouched). CI run 39765708214: all jobs refused — GitHub account billing lock (KI-14). Docker Desktop crashes at start on this PC (KI-14); I closed the instance I started, changed nothing in its data.
- Verified locally: check.sh GREEN 100 s after the CARGO_TARGET_DIR change; package.sh → dist/LGDuel-windows-x64.zip (packaged host + bot + headless client ONLINE CHECK OK).
- PLAN reconciled: 6.1, 6.3, 7.2, 8.3, 11.1 ticked (done by v0.2 work; references in PLAN). 8.2 not ticked (gun-model option and input display missing = 8.4/8.5).


## 2026-10-22 — iteration 56: PLAN 6.2 challenges
- net: protocol v6, `RosterEntry`, `Control::{Roster, Challenge, Respond}`, status challenge fields (round-trip tests); NetClient `roster`, `messages`, `challenge`, `respond`.
- server: `Client.{challenge, roster_sent}`, `challenge`/`respond`/`tell`, expiry each tick, roster sent on change in 16-entry chunks.
- client: NetSession toast (4 s), challenge-aware `status_text`, `roster`/`incoming_challenge`/`challenge`/`respond`; menu `PlayerRow`, players grid + Accept/Decline in the online pause menu (window 380 wide when listed); `--menu pause-online` sample render (in check.sh).
- Verified: arenas test `challenge_accept_decline_and_expiry` (roster of 4 with arenas; challenge visible to both; accept → both in the empty arena 3rd, left-behind pair re-paired; decline → challenger told "BOT1 DECLINED"; unanswered → cleared after 15 s with "CHALLENGE EXPIRED", nobody moved); pause-online screenshot inspected; check.sh GREEN 101 s.


## 2026-10-22 — iteration 57: PLAN 6.4 + 9.3 end-of-fight summary
- client/fight.rs: `FightTracker` keeps everyone's stats at fight start; on a `FightEnd` involving the viewed player (own or spectated) it builds `FightSummary` {won, secs, acc, opp_acc, dealt, taken} from the stat differences, shown 4 s; `FrameView.stats` (local, online, spectator views); `hud::centre_block` draws it above the crosshair; App feeds it every frame. `--summary-demo` renders a sample (in check.sh). HUD fight timer now "FIGHT 4.0 SEC" (in the pixel font "4.0S" read as "4.05").
- Per-player stats were already complete: HUD LG %, DMG dealt/taken, W-L, fight timer; server stats file with fight_ticks.
- Verified: fight tests (summary counts only this fight from both sides, ignores other players' fights, disappears after 4 s); summary screenshot inspected (3 centred lines, readable); check.sh GREEN 93 s. PLAN 9.3 ticked with it.


## 2026-10-22 — iteration 58: PLAN 6.5 admin console, RCON, hot reload
- server/admin.rs: `Server::{command, config, apply_config, rcon}`; `config_path`, `rcon_password`, rcon lockout map. host.rs: `Admin {config_path, console}` passed to `run_config`, stdin console thread. lgduel-server `rcon [--password PW] ADDRESS COMMAND…` (or LGDUEL_RCON_PASSWORD). net: protocol v7 Rcon/RconReply. DEPLOY.md admin section; example config `rcon_password`.
- Verified: server tests/admin.rs (status/help/unknown; get/set across all arenas incl. browser info; refused: unknown key, bad value, physics, max_arenas, port; kick → client Disconnected("kicked: too good"); say reaches clients; reload: no file / applies / refuses physics / rejects unknown key; RCON with password, wrong password silent, lockout after 5, unlocked after 61 s, off without password). Real runs: piped console commands into lgduel-server; `lgduel-server rcon` status + kick of a live lgduel-bot ("lgduel-bot: disconnected: kicked: bye"), wrong password → "no reply" exit 1. check.sh GREEN 108 s.


## 2026-10-22 — iteration 59: PLAN 7.3 LAN discovery
- net: `UdpTransport::set_broadcast`, `Pinger::discover` (open nonce, any responder, `discovered`), protocol v8 `ServerInfo.instance`. server: instance = keyed hash of its secret. client browser: broadcast + loopback discovery on refresh, `lan_rows` (dedupe by instance, loopback first, skip listed), `Row.lan`, "LAN" tag in the table; Host screen text fixed (stray spaces) and points to DEPLOY.md; "…" → "..." in UI strings.
- Verified: client test `lan_discovery_finds_an_unlisted_server_once` (server bound 0.0.0.0 on a random port answered on 127.0.0.1 and via broadcast from the 172.31.96.1 adapter → one LAN row; once listed as 127.0.0.1 → no LAN row) — first version failed exactly there, which is why rows are keyed by instance; net info test now covers worst-case name + map lengths with the instance field (response ≤ 128-byte request). Real run: lgduel-server on 27961 (not listed) appeared in the Online menu screenshot with ping, map, rules, Join, LAN. check.sh GREEN 107 s.


## 2026-10-22/23 — iteration 60: PLAN 7.4 + 7.5 join codes, hole punching, relay
- net: MemNet NAT simulation (`endpoint_behind_nat`, `NatKind::{Cone, Symmetric}`, `nat_dropped`); rendezvous.rs (codes, `Rdv` messages, `RelayTransport` plain/host/join, `forcing_relay`). server: relay.rs `Rendezvous` + `run_relay`, `lgduel-server relay`; `rendezvous` setting / `--rendezvous`; host loop prints "JOIN CODE LG-XXXX", `Admin.join_code`, `Server.join_code` in status. client: NetSession over RelayTransport (`connect_code`, `join_progress`, `join_failed`), Online screen Join code + via field, Host screen Join code checkbox, code in the Esc-menu hosting info, Connecting screen progress, `--join/--rendezvous/--force-relay`; bot `--join/--rendezvous/--force-relay`. tools `p2p-smoke`; check.sh p2p stage (KI-2 resolved). Deploy: lgduel-relay.service, installer enables it + ufw 27950, DEPLOY/PLAY docs, servers.toml `rendezvous`.
- Headless online check now waits ≤ 2 s for a living opponent before the final picture (it failed 2 of 3 times when a session ended right after a frag — a check artefact, not netcode).
- Verified: rendezvous unit tests (codes, padding); MemNet NAT test; server tests/p2p.rs — cone NATs connect directly by code (no relay session), symmetric NATs fall back to the relay (NAT drops > 0, 2275 relayed datagrams, players fight), unknown code → "no game with that join code"; real runs: relay + host + bot by code (direct) + client --force-relay (relayed, "ARENA 1 - VS CODEBOT", 3/3 ONLINE CHECK OK); windowed `--host-game --rendezvous` got LG-JPCB and showed it; Online/Host menu screenshots inspected; check.sh GREEN 125 s with P2P SMOKE OK.


## 2026-10-23 — iteration 61: PLAN 7.6 master list + browser filters
- net: `Rdv::Register{listed}`, `ListRequest/ListChallenge/List`, `RelayTransport::listed`, `Pinger::{master, request_list, public}`; protocol v9 `ServerInfo.region`. server: relay keeps `listed` per host, cookie handshake for lists (`lists_sent`, `listed()`); settings `public`, `region` (restart-only for `public`). client: Browser `set_master` (resolved in the background like servers), public rows; settings `BrowserFilter` + `allows`; Online screen filter row, Region column, PUBLIC tag, hidden count; headless menu passes the master too.
- Verified: p2p test `master_lists_public_servers_only_to_proven_addresses` (2 hosts, 1 listed; cookie-less request → challenge no larger than the request, no list; browser handshake → list of 1 → pinged with name and region); settings `browser_filter` test; real run: relay + public server (EU-TEST, port 27980 outside LAN discovery) appeared in the Online menu screenshot with ping, region, Join, PUBLIC; check.sh GREEN 126 s.


## 2026-10-23 — iteration 62: PLAN 8.1 console, cvars, profiles
- client/console.rs (`Console`, `cvars`, `set`, `run` with exec/writeconfig/aliases/actions); menu: console field, `is_open` includes it, `draw_console`, `Action::ConsoleLine`; main: ` toggles (passed through egui like Esc), `console_line` (applies + saves settings, acts on connect/join/practice/disconnect/quit), `quit_requested`; `--menu console` sample render (check.sh). PLAY.md console section.
- Verified: console tests (set by full key / alias / Quake style, nested crosshair key, bool, colour list, text with spaces; sanitising shown (fov 500 → 130); wrong type and unknown key refused; cvars filter; join action; comment lines; writeconfig → exec round trip reproduces the settings exactly, path-like names refused, missing file / no folder errors); console screenshot inspected (opaque, readable); check.sh GREEN 110 s. Not automated: typing into the real window (needs a human).


## 2026-10-23 — iteration 63: PLAN 8.4 + 8.5 (+ 8.2)
- hud: `InputView`, `input_display` (setting `hud.input_display`); effects: `GunModel` (setting `effects.gun`); scene: `view_camera`, `push_gun`, `hud_mesh(…, input)`; SpectateView carries buttons; App feeds keys / watched buttons; Visuals tab: Input display checkbox, Gun model None/Small/Normal; `--gun`, `--input-display`; check.sh renders both; Camera derives PartialEq.
- Verified: scene tests (`no_screen_shake_and_no_blood`: same camera and byte-identical dynamic mesh with damage/kill events, gun on and off; `gun_model_options`: +72 vertices (two boxes) for small/normal, none when dead); screenshot inspected (gun lower right along the beam; input display W/D/LG lit for the sample input); check.sh GREEN 111 s. PLAN 8.2 ticked (every §1 visibility option now in Settings).


## 2026-10-23 — iteration 64: PLAN 9.1 + 9.2 demos
- net/demo.rs (format + tests); client: LocalSession `from_game`, `start/stop_recording`, `step_with_cmds`, `due_ticks`; NetSession recording (snapshots + cmds); demo_play.rs (`DemoPlayer::{Local, Net}`, `Verdict`, `run_to_end`, frame_view with recorded angles, input_view); Session::Demo; App recording (`--record`, console record/stoprecord, saved on session end), `play_demo` (console playdemo, `--play-demo`), input overlay forced in demos; headless `run_headless_record` / `run_headless_demo`; console `demo_path`; PLAY.md demos section; check.sh record + replay stage.
- Verified: demo format round trip / garbage / truncation / version; `local_demo_replays_bit_exactly` (3000 ticks of a real fight on arena_pillars → Match with identical stats; one tampered yaw → Mismatch); headless: 20 s arena_tiers practice recorded (dealt 335 / took 35) → replay Match, end screenshot shows "REPLAY MATCHES THE RECORDING" + input overlay; online demo recorded from a real server session (999 frames) and played back (screenshot: the duel from the player's view); check.sh GREEN 128 s.


## 2026-10-23 — iteration 65: PLAN 10.1, 10.2, 11.2, 11.3
- 10.1: windowed 1920x1080 practice duel, vsync off, RTX 4060 / Ryzen 7 9700X: avg 4138 fps (p50 0.20 ms, p99 0.58 ms); arena_tiers + gun + input display 3933 fps. Target 500+ met. 10.2: input latency path documented; sizes release vs dist; startup ~650 ms to first frame, server ~30 ms. docs/PERF.md. `[profile.dist]`, package.sh + release workflow use it.
- 11.2: HOSTING.md, CONFIG.md (+ `lgduel --print-settings`); 11.3: MAPS.md (second/third maps exist since M3.3). README links, docs shipped in the zip.
- Verified: package.sh with dist profile → PACKAGE OK (packaged host + bot + client smoke); check.sh GREEN 112 s.

## 2026-10-23 — status: every remaining item is blocked on the user
- Left: M5.1/M5.2 + 7.7 (CI / Docker verification), 7.8 (Linux check.sh, KI-1), M5.3 (v0.2.0 release with the official server), 11.4 (DoD run needs Linux green). Pushed v0.2-dev @ 3e49cb9; CI run 39801090961 again refused: "account is locked due to a billing issue". Docker Desktop on the dev PC still crashes at start (KI-14).
- Needed from the user: fix the GitHub billing lock (or start Docker Desktop after a reboot), and the VPS address (DEPLOY.md) for the shipped server list + rendezvous.

## 2026-10-23 — iteration 66: visuals T1 material tags
- world.rs/brush.rs: `material` keys + fields; World literals gained `materials: Vec::new()`; MAPS.md section.
- Verified: new parse test for all four entry kinds, content-hash test extended (tags don't change it), determinism golden hash 0x9e74a8ea7918ee78 unchanged, check.sh GREEN 133 s.

## 2026-10-23 — iteration 67: visuals T2 textured world pipeline
- textures.rs (Image, mips, TextureSet, upload), render.rs (box_faces, WorldVertex, Surface/Look/WorldStyle/ClassicStyle, planar_uv, world_mesh_textured, world pipeline + texture bind group, set_world(device, queue, world, style)); callers updated.
- Verified: classic textured mesh == old mesh for all built-in maps, planar_uv test, mip/dedupe tests, check.sh GREEN 112 s with no pixel-assertion changes, 4552 fps at 1080p (arena_tiers).

## 2026-10-23 — iteration 68: visuals T3 themes
- theme.rs (Theme, Material, ThemeStyle, procedural textures, available()), settings `video.theme`, menu combo, `--theme`, App `style` + `refresh_style`, headless uses the theme's sky and per-theme checks, check.sh grid render.
- Verified: 7 theme tests (classic = classic look everywhere, grid layers by orientation, resolution order with per-map override and unknown tag, typo rejection, missing texture reported, procedural textures deterministic with pinned fingerprint, style key stable/different); grid screenshot inspected (textured floor/walls/ceiling, enemy clearly visible, 593 colours); windowed run OK; 6858 fps at 1080p with grid; check.sh GREEN 113 s.

## 2026-10-23 — iteration 69: visuals T4 user themes
- textures.rs (PNG decode with limits + resize, safe_relative, load_png_file, encode_png_rgb), theme.rs (load, find_theme_dir, available, search_dirs, example_theme_files), `--theme-dir`, `--make-example-theme`, console `reloadtheme`, Menu.themes, assets/themes/example, THEMES.md, package/release copy the example, check.sh renders it.
- Verified: PNG decode tests (all colour types, shrink/grow, 2049 px rejected, garbage, truncated), path safety, broken-theme fallbacks (unknown name, bad TOML, `../`, missing, absolute, too many, oversized file), committed example == generator output and loads; screenshot inspected (brick walls, tiled floor, enemy clearly visible); exe-folder search verified; check.sh GREEN 114 s.

## 2026-10-23 — iteration 70: visuals T5 character models
- render::push_obox (gun refactored onto it), models.rs, scene_mesh takes `&ModelSettings`, settings `[models]`, Visuals tab (model, forced colours, skin pickers), `--enemy-model/--skins/--enemy-distance`, check.sh humanoid + skin renders.
- Verified: 6 model tests (hitbox containment, exactly three forced colours / skin has none of them, classic == original, legs move while torso/head don't and yaw turns the model, crouch lowers it, settings round trip); close-up screenshots inspected; check.sh GREEN 115 s; 6675 fps at 1080p with grid + humanoid.

## 2026-10-23 — iteration 71: visuals T6 docs and wrap-up
- PLAY.md (theme, enemy model, forced colours), CONFIG.md (`video.theme`, `[models]`), docs/PERF.md (grid + humanoid: 6675 fps), console test for the new cvars (incl. an invalid enum value is refused). Visual customisation plan T1–T6 complete (STATE/HANDOFF_VISUALS.md).
- Verified: check.sh GREEN (see log); determinism golden hash unchanged since T1.

## 2026-10-23 — iterations 72-80: Linux, share codes, look & feel (STATE/HANDOFF_LINUX_LOOK_SHARE.md)
- Part A: native Linux build verified in WSL2 Arch (full check.sh green there), XDG theme dirs, window app id, `--doctor`, generated icons + .desktop, split PKGBUILD (lgduel, lgduel-server; sysusers + systemd units), `check-pkgbuild.sh`, INSTALL.md, release/CI workflows (YAML parses; cannot run, billing lock).
- Part B: `LGP1-` profile codes (zlib + CRC32 + base64url, whitelist, caps), presets, `.lgtheme` packs, Share tab, SHARING.md.
- Part C: MSAA (`video.msaa`, `--msaa`), window title (map + ping), HUD auto-scale + menu scale, crosshair shapes/outline/hit colour, sound set, menu accent style, sky gradient + edge outlines + midnight/paper/ember themes, TAB scoreboard.
- Verified: `makepkg` in WSL Arch built lgduel + lgduel-server; installed, `lgduel --version`/`--doctor` ran; every milestone check.sh GREEN on Windows (last 127 s); MSAA 4x render has 58 distinct colours vs 12 and 4146 fps at 1080p windowed; scoreboard screenshot inspected (panel high on screen, enemy/crosshair/beam still pass).

## 2026-10-23 — KovaaK's import K1 + K2 (continued on Linux/CachyOS)
- `crates/client/src/kovaaks.rs`: install discovery (libraryfolders.vdf parser, Windows/Linux/Flatpak roots, manual `kovaaks.path` / `--kovaaks-dir`), theme JSON → LGDuel theme conversion, `install_theme`, `import_themes`, `--kovaaks-report`, `--import-kovaaks-theme`. New deps `serde_json`, `lewton` (for K3). `offscreen::srgb8_to_linear`.
- Verified: 16 new tests; baseline `check.sh` GREEN on Arch (CachyOS) before the change; `--kovaaks-report` found the real Steam install through libraryfolders.vdf; imported all 113 real themes (0 load problems, 113 render headless); fixture import renders with its orange/green enemy colours (RENDER CHECK OK).

## 2026-10-23 — KovaaK's import K3 + K4 + K5 + K7
- `sound_file.rs` (Ogg Vorbis via lewton, hand-written WAV, mono, <= 2 s, resample), `audio::CustomSounds` + hit voice cap, `audio.hit_sound` / `kill_sound_file`, console `hitsound`, `kovaaks::{import_sounds, run, scan}`, console `kovaaks …`, Share tab KovaaK's section, `--import-kovaaks-sounds`, `check.sh` KovaaK's stage, KOVAAKS.md + CONFIG/README/PLAY/SHARING links.
- Verified: unit tests (WAV bit depths/float/extensible/truncated/oversized chunk/silent/ADPCM, broken ogg, resample, voice cap, sound import + case-insensitive lookup, console `hitsound`, sanitize, share code excludes sound files); ignored test `real_install_sounds_decode` on the real Proton install: 438 of 441 decode, the 3 refused are correct (MS ADPCM, two silent files); real import of the hit/kill sound and `--sound-test` opened audio with them; Share tab screenshot inspected.

## 2026-10-25 — Direct codes + automatic router port mapping
- net: `portmap.rs` (UPnP IGD + NAT-PMP, std only, `Probe`/`open`/`Mapping::{renew,close}`), `rendezvous::{format_direct, parse_direct}`. server: `server.upnp`, `--upnp`, `Admin.direct_code`, port-mapping thread in `run_config` ("DIRECT CODE ..." line). client: `host.upnp` + "Open port" row, Esc-menu direct code, Join code field accepts both code kinds (`join_session`). bot `--join` accepts direct codes. tools `p2p-smoke` joins the real client by direct code too. host.sh/host.ps1 pass `--upnp`. Docs: PLAY, HOSTING, CONFIG, README; D66.
- Verified: unit tests with fake UPnP routers (plain + chunked replies, 725/718 quirks, CGNAT undone, hostile LOCATION ignored, oversized reply refused) and a fake NAT-PMP router; direct-code round trip + single-typo detection; `p2p-smoke` (direct bot, relayed client, direct-code client all OK); real run on this network: reports "UPnP is probably switched off in the router's settings; NAT-PMP: no answer" (accurate: the router ignores both). Not verified: a real router that accepts the mapping.

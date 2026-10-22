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

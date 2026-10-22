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

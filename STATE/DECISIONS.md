# DECISIONS

## D1 — Tech stack (2026-10-22)
Rust workspace, as recommended:
- `sim` (pure deterministic lib), `net` (protocol/prediction/lagcomp lib, transport-agnostic),
  `server`, `client` (binary name `lgduel`; will carry subcommands such as `lgduel host`), `bot`, `tools`.
- Renderer: **wgpu + winit** directly (not Bevy): faster compiles, full control over present mode
  and frame pacing for the 500+ FPS / low-latency goal, and a straightforward offscreen path for
  headless screenshots in check.sh.
- Transport: plain UDP via std `UdpSocket` with our own small reliable/unreliable channel layer.
  We need custom sequencing, redundant inputs and in-process netem anyway, and a library whose model
  differs from ours would get in the way. Revisit only if this proves costly.
- Config: TOML via `serde` + `toml`.
- Toolchain pinned to rustc 1.99.1 in rust-toolchain.toml (determinism, no churn).
- check.sh is bash; on Windows it runs under Git Bash. Linux is the reference platform for DoD.

## D2 — Determinism policy (2026-10-22, refine in PLAN 2.1)
- Sim uses `f32` with controlled operations: no `mul_add`/FMA, no platform libm transcendentals in
  the tick path (angle sin/cos via our own fixed routine on quantized angles), fixed iteration order,
  no HashMap iteration in sim. Client and server run the identical sim crate, so prediction matches
  bit-for-bit; cross-platform agreement is verified by state hashes in tests.
- Fallback if cross-platform float drift is observed: switch the sim to fixed-point.
- Q3-style velocity snapping (round to integers at end of pmove) is a candidate: it matches the
  reference feel and aids determinism. Decide in 2.6 and record here.

## D3 — Physics source plan & starting values (2026-10-22, VERIFY in Phase 2)
Sources to check: ioquake3 `code/game/bg_pmove.c` and `bg_public.h` (GPLv2: read for values and
behaviour only; our implementation is written from scratch, no code copied), Quake Live server cvar
docs (pmove_*, g_speed, weapon_reload_lg, g_damage_lg), x76.gg wiki for strafe-aim ratios.

| Value | Start | Source to confirm |
|---|---|---|
| gravity | 800 u/s² | Q3 g_gravity |
| max ground speed | 320 | Q3 g_speed |
| jump velocity | 275 (Q3: 270) | QL pmove_JumpVelocity |
| ground accel | 10 | pm_accelerate |
| air accel | 1 (VQ3/QL, no CPMA air control) | pm_airaccelerate |
| friction | 6 | pm_friction |
| stop speed | 100 | pm_stopspeed |
| step height | 18 | STEPSIZE |
| crouch speed scale | 0.25 | pm_duckScale |
| hitbox | mins (-15,-15,-24) maxs (15,15,32); crouch maxs.z 16 | bg_pmove PM_CheckDuck |
| LG refire | 50 ms | weapon_reload_lg |
| LG damage | 7 (start; 6–7 range) | g_damage_lg |
| LG range | 768 | QL LG range |
| LG knockback | small (Q3 knockback scales with damage) | g_knockback |
| health | 100 | — |
| armor | off | — |
| tick | 125 Hz | sv_fps / pmove_msec 8 |
| lifesteal | 100% (vamp default) | x76 vamp servers |

Final values and exact citations get recorded here once verified (PLAN 1.4 / 2.x).

## D4 — Legal
No id Software / Quake Live assets, maps, models, sounds or code. GPL Quake sources are consulted
only for numeric constants and observable behaviour; the implementation is original. All assets are
original or CC0/MIT.

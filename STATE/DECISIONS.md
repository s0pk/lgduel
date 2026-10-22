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

## D5 — Config shape (2026-10-22)
- One `GameConfig` struct in `sim` (parsing a string is pure; file reading stays in server/client).
  Server-only settings (ports, name, region, rcon) will live in a separate server struct that embeds `GameConfig`.
- `deny_unknown_fields` everywhere: a typo in a server config is an error, not a silent default.
- `max_health` = 100 caps lifesteal healing (vamp heal never overheals). Unverified against x76 servers;
  revisit in 2.12 if the community format allows overheal.
- `lg.knockback = 1.0` is a placeholder scale; real formula and value are decided in PLAN 2.11.
- Default `hit_mode = anti_lag`; both modes are first-class settings shown in the server browser later.

## D6 — Angles & hashing (2026-10-22)
- View angles are `Angle(u32)`, 2^32 per 360° (≈8.4e-8°): the client sends exact angles (no Q3 16-bit
  ANGLE2SHORT loss), so sub-pixel mouse input survives the wire.
- Trig is our own: integer octant reduction + degree-15/16 Taylor in f64, then cast to f32. Same bits on
  every IEEE platform. Error < 1e-14 vs std.
- State hash: FNV-1a 64 over exact bit patterns (not `std::hash`, whose output may change between releases).

## D7 — Movement model details (2026-10-22)
- pmove follows the Quake 3 movement structure (friction → accelerate → clip to ground → step-slide;
  air: accelerate with air_accelerate, trapezoidal gravity, 4-bump / 5-plane slide, step-up). Written
  from the documented behaviour; no code copied.
- Deviation: the walk-move "clip velocity to ground then restore speed" step uses an exact projection
  (overbounce 1.0) instead of OVERCLIP 1.001. With OVERCLIP + renormalise, a residual +1 u/s vertical
  velocity after landing (from velocity snapping) is restored every tick and never decays.
- `snap_velocity` rounds to nearest (ties away from zero). Original Q3 C code truncates, but the x86
  builds people actually played rounded to nearest (fistp), which is what produced the familiar
  125 Hz feel. Revisit against golden jump numbers in 2.9.
- `auto_hop = true` default (Quake Live lets you hold jump to bunny hop); Q3 behaviour available via config.
- Jump sets vertical velocity to jump_velocity (no additive/ramp jumps, no double jump; VQL-style).
- Open question: Quake Live may use step height 22 rather than Q3's 18 — verify (KI-4).

## D8 — Golden movement numbers (2026-10-22, 125 Hz, default config)
Pinned in `crates/sim/tests/movement_golden.rs`. Changing physics must update these deliberately.
- Standing jump: apex 50.342u above rest, airtime 92 ticks (736 ms). Ideal parabola 47.27u / 86 ticks;
  the excess comes from velocity snapping (round-to-nearest), matching the known 125 Hz Q3 behaviour.
- Ground accel 0 → 320: 19 ticks (152 ms).
- Circle jump (12 ticks optimal ground turn + jump): takeoff 378.3 u/s.
- Perfect strafe jumping, 6 s from 320: 1145.7 u/s (speed² grows ~1632 per tick with air accel 1,
  i.e. matches the closed form; humans get far less because of non-optimal angles).
- Strafe-aim ratios fall out of `cmd_scale` (diagonal speed = straight speed): 0.000 / 2.000 / 0.294 / 1.706 v
  (wiki: 0.00 / 2.00 / 0.29 / 1.71). The HSW 0.001 deviation is velocity snapping on the diagonal.
- Web search for Quake Live pmove cvar defaults (step height, jump velocity, LG damage/knockback) found no
  authoritative source; values stay configurable, KI-4 remains open.

## D9 — Combat model (2026-10-22)
- LG refire uses a µs weapon timer that carries negative remainder (Quake style): at 125 Hz, 50 ms cells
  alternate 6/7-tick gaps and average exactly 50 ms. Releasing fire clamps the timer to ≥ 0.
- Hitscan from eye (origin + view_height) along view forward, range 768; nearest of world/target boxes
  wins. `hitscan()` takes target boxes explicitly so lag compensation can pass rewound boxes.
- Knockback: velocity += dir · damage · 5 · lg.knockback (5 = Quake g_knockback 1000 / mass 200).
  At default 1.0 → 35 u/s per cell. Unverified vs QL (KI-4).
- Armor (off by default) absorbs ceil(66%) like Quake.
- Lifesteal is based on damage actually dealt (health removed, capped at the target's remaining health,
  plus armor absorbed). Integer percentage with a hundredths remainder carried per attacker, so over any
  sequence `lifesteal_earned == floor(total_dealt * pct / 100)` exactly; healing is capped at max_health
  (`healed` ≤ `lifesteal_earned`).
- No player-vs-player collision yet (players pass through each other) — KI-6.
- Respawn delay rounds up to whole ticks; spawn = point farthest from living opponents.

## D10 — Rules & isolation (2026-10-22)
- A *fight* runs from the tick all players are alive to the next frag; `FightEnd{winner, loser, ticks}`
  is emitted in both modes and feeds W/L + fight-length stats.
- Vamp (default): no frag limit; the winner is reset to spawn health/armor (`reset_winner_health`, default
  true) so every fight starts even. Classic: frags count to `frag_limit` → `MatchEnd`, scores reset.
- **Pair isolation = one `Game` instance per duel arena.** The server will host many `Game`s (one per
  pair) plus lobby/spectators, so cross-pair damage/collision is impossible by construction (Phase 6).

## D11 — Determinism verification (2026-10-22)
- `sim::math::atan2_det` / `look_angles`: deterministic inverse trig (basic ops only, < 1e-13 vs std), so
  bots/tests/lag-comp aim never depend on platform libm.
- Determinism test records a 60 s seeded duel's inputs and replays them; hashes must match each other and
  the pinned golden hash 0x9e74a8ea7918ee78. Running the same test on Linux (KI-1) is the cross-platform
  drift check. Any intentional sim change updates the pin (note it in PROGRESS).

## D12 — Client rendering base (2026-10-22)
- wgpu 30 + winit 0.30, `ControlFlow::Poll`, redraw requested every frame (uncapped). Present mode:
  vsync off → Immediate > Mailbox > Fifo; vsync on → Fifo. `desired_maximum_frame_latency = 1`.
- Optional `--fps-cap N` uses sleep-based pacing (good enough for now; a spin-wait tail can be added in
  Phase 10 if cap accuracy matters).
- `lgduel --frames N` renders N frames then prints a `FRAMESTATS` line (p50/p99/max, avg fps, present mode).

## D13 — Renderer (2026-10-22)
- Single flat pipeline: position + colour vertices, depth test, no lighting. Per-face brightness
  (top 1.0, ±x 0.85, ±y 0.72, bottom 0.55) keeps geometry readable while staying fullbright.
- Camera: Quake axes → view space via angle_vectors; own `perspective_rh` (0..1 depth; glam 0.34 moved
  its projection API). FOV: Quake-style horizontal FOV at 4:3, clamped 10–130, Hor+ for wider aspects.
- Headless: `lgduel --headless --screenshot P` renders offscreen (hardware adapter, software fallback),
  reads back, saves PNG, asserts ≥4 distinct colours and floor colour ≥5% of pixels; exits 1 otherwise.

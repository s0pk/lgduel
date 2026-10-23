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

## D14 — Input & local loop (2026-10-22)
- Mouse: winit `DeviceEvent::MouseMotion` (raw counts, no OS accel) while the cursor is grabbed
  (Locked, falling back to Confined). cm/360 + DPI → angle units per count; f64 remainders carried so
  sub-count motion is never lost; pitch clamps to ±89° (remainder dropped at the clamp).
- View angles used for rendering are the latest mouse state (not tick-quantized); each tick's UserCmd
  carries the exact `Angle` at the time the tick is built.
- Local sim runs through `FixedStepper` at the config tick rate; own eye is interpolated between the last
  two ticks (teleports > 128u snap). Frame rate never changes sim results (tested: 1000 fps vs 37 fps
  give identical state hash).

## D15 — Player models & forced colours (2026-10-22)
- Model = the hitbox itself, split into lower (0–45% height) / upper (45–85%) / head (85–100%) bands,
  plus a small facing nub at head height. Solid unshaded colours (no face shading) = maximum visibility,
  and what you see is exactly what you can hit. Crouch shrinks the model with the hull.
- Default forced enemy colours: head yellow (1,1,0), upper green (0,1,0), lower cyan (0,0.6,1) —
  configurable in Phase 8.
- Headless check now asserts each forced colour appears (≥50 px) on an opponent at 768u.

## D16 — LG beam & sparks (2026-10-22)
- Beam styles: none / thin (1.2u) / thick (3.5u) / zigzag (jagged, ~24u segments, ±5u jitter); camera-facing
  ribbons in the flat pipeline. Default thin, colour (0.55, 0.8, 1.0).
- Own beam is predicted every render frame from the latest view angles (not tick-quantized) via
  `sim::game::hitscan` against interpolated enemy boxes, drawn from a muzzle offset (14 fwd, 6 right, 8 down)
  so it is visible. Enemy beams come from their latest buttons/angles. Purely visual — hits are authoritative.
- Sparks: 6 streaks at the impact, reflected into the hemisphere facing the shooter so the target never
  hides them; on/off + size (0–32u half-size, default 3). Drawn only when something was hit inside range.
- Headless check places the opponent at 400u and asserts beam/spark pixels; `--beam none --no-sparks` must
  draw zero beam and zero spark pixels.

## D17 — Audio (2026-10-22)
- cpal 0.18 output stream (default device/config, f32 or i16). Hit sound synthesised at the device rate:
  1.9 kHz + 2.85 kHz sines, 1 ms attack, e^-90t decay, 45 ms, peak 0.9 — original, no sample files.
- Main thread → audio thread via mpsc channel (one message per hit, carrying gain); the callback mixes
  voices without locks. No device / muted → silent handle, the game still runs.
- One hit sound per own Damage event (LG cell that hit); `--volume 0..1` (default 0.7), `--no-sound`,
  `--sound-test` (plays 5 hits on the real device, exit 1 if no output).

## D18 — HUD (2026-10-22)
- Second pipeline in the same pass: vertices already in NDC, depth compare Always, no depth write → always
  on top, no extra pass. Text from an original built-in 5×7 pixel font (no font files/licences).
- Layout: health big bottom-left; LG %, DMG dealt/taken, W-L, fight time top-left; opponent health
  top-right; crosshair (arms/gap/thickness/dot/colour, default pink so it differs from every scene colour);
  1-px dark drop shadow on all text.
- Damage numbers: hits less than 0.25 s apart merge into one running total that rises 40u and vanishes
  after 0.6 s (a stream of 7s at 20 Hz was an unreadable stack).
- Headless run now really fires: fixed virtual 2 ms per frame (300 frames = 0.6 s = 12 LG cells),
  knockback off so the scene is stable; checks crosshair/HUD text/damage-number pixels.

## D19 — Bots (2026-10-22)
- `lgduel_bot` lib (+ bin). Perception: opponent position from `reaction_ms` ago plus `lead`×velocity×reaction.
  Aim: proportional controller (`gain`/tick) capped by `turn_speed`, plus uniform `noise_deg` jitter. Presets
  aimbot / hard (120 ms) / medium (180 ms) / easy (250 ms).
- Strafe patterns relative to the line between the players: still, random A/D (hold min..max ms, optional
  jumps), mirror, anti-mirror, HSW mirror, HSW anti (diagonal with range-band forward/back). Range keeping
  via min/max band. Seeded PRNG → same seed + same game = same inputs.
- Measured bot-vs-bot (8 s, 200–400u band, zero reaction): relative lateral speed mirror 0.11v, anti 1.86v,
  HSW 0.31v, HSW anti 1.74v (ideal 0 / 2 / 0.29 / 1.71; transitions between directions account for the gap).

## D20 — Transport & packet layer (2026-10-22)
- `Transport` trait; `UdpTransport` (std non-blocking socket; ignores Windows WSAECONNRESET from stale ICMP)
  and `MemNet` (in-memory, manual µs clock, seeded latency/jitter/loss, deterministic — basis of netem tests).
- `Connection`: header magic/seq/ack/32 ack bits (each packet acks 33); reliable ordered messages resent
  (RTT×1.25, ≥30 ms) in later packets until a packet carrying them is acked; unreliable payload per packet
  with `newest` flag; MTU cap 1200 bytes. RTT/jitter: TCP-style smoothing (1/8, 1/4).
- Loss = packets that fall > 32 behind the newest ack without being acked (and un-counted if a straggler
  ack arrives later).

## D21 — Protocol (2026-10-22)
- Handshake (connectionless, first byte 0xFF): ConnectRequest{version, nonce} (padded ≥ Challenge size: no
  amplification) → Challenge{nonce, cookie=keyed hash(secret, addr, nonce)} → ConnectResponse{…, cookie,
  name ≤ 24} → Accept{player id, tick rate, gameplay config TOML} / Reject{reason}. Spoofed sources never
  get a slot (they can't see the cookie).
- Inputs (unreliable): last 8 UserCmds per packet (exact u32 angles) + newest snapshot tick seen. Server
  `InputBuffer` takes each tick's command once, sanitises it, counts duplicates/late, and reports lead.
- Snapshots (unreliable): full state at `game.tick`, f32 as exact bits, events since the last snapshot, and
  the receiving client's last consumed command tick. 173 bytes for 2 players before delta compression.
- Control (reliable): Disconnect, ServerEvent.

## D22 — Delta snapshots (2026-10-22)
- Per-player 14-bit changed-field mask vs. the baseline snapshot the client last acked (via Inputs.ack_snapshot);
  players absent from the baseline go as full records (mask = 0x8000). Floats compared and sent as exact
  bits (no quantisation) so prediction stays bit-exact. Events always sent in full.
- Server keeps a `SnapshotHistory` (64) per client; client keeps its received history (64). Missing/wrong
  baseline → decode returns None (client simply waits for the next one; server falls back to full when the
  acked tick has left its history).
- Measured: active 2-player fight with 20% snapshot loss → 41% of full size (118 KB vs 286 KB over 1500
  snapshots ≈ 79 B/snapshot ≈ 9.8 KB/s at 125 Hz); unchanged state 25 B.

## D23 — Server (2026-10-22)
- `lgduel_server` lib (generic over Transport) + `lgduel-server` bin. Config = one TOML: `[server]` (name,
  bind, port, max_clients, snapshot_every, timeout_ms, map) + gameplay sections (GameConfig); the example
  config parses to the defaults (tested).
- Tick: consume each client's command for exactly this tick from its InputBuffer; if missing, repeat the
  client's last *real* command (startup ticks before the first command don't count as missing). Snapshots
  every `snapshot_every` ticks as deltas against the client's acked baseline; events accumulate per client
  between snapshots.
- Real-time loop: FixedStepper + ≤1 ms sleeps (socket polled ≥1 kHz). Timeout 5 s; Disconnect control msg.

## D24 — Clock alignment (2026-10-22)
- `net::client::NetClient`: handshake with 250 ms retries; per-snapshot server-tick estimate = newest snapshot
  tick + (time since arrival + RTT/2)/tick. Target client tick = estimate + lead, lead = (RTT/2 + 2·max(RTT
  deviation, snapshot arrival jitter))/tick + 1 safety tick. Client clock dilates ≤ ±5% toward the target;
  snaps if off by > 250 ms. Commands are generated for each client tick as the clock crosses it.
- Measured (MemNet): 50±10 ms → lead 8.8 ticks, server buffer ≤ 6, 0 late cmds/10 s; after switching to
  150±10 ms → lead 21.6 ticks, 0 late cmds/10 s; 20 s → client 2500 ticks = server 2500 ticks.

## D25 — Prediction (2026-10-22)
- `net::predict::Predictor`: own PlayerMove predicted by replaying every sent command after the snapshot's
  `last_cmd` through the same `pmove`; on each newest snapshot the state is rebuilt from the authoritative
  one (reconciliation). Visual correction kept as an offset decaying at 15/s; corrections > 64u (teleport,
  respawn) snap. Own LG beam is drawn client-side from the predicted eye + current view (as offline).
- Error metric: predicted origin after command T vs. authoritative origin in the snapshot whose last_cmd = T.
- Threshold (PLAN 4.11): mean < 0.5u at 0% loss. Measured: exactly 0.0 (bit-exact) at 0 ms, 50 ms,
  100±20 ms and 50±10 ms with 5% loss.
- NetClient seeds RTT from the handshake (ConnectResponse → Accept) until acks exist; without it the first
  commands were stamped with ~1-tick lead, arrived late, and caused a one-off 271u misprediction at 50 ms.

## D26 — Remote-player interpolation (2026-10-22)
- Render remote players at `snapshot_clock − delay`. The snapshot clock = newest snapshot tick that could
  have arrived by now on the fastest path seen in the last 2 s (min of receive−tick offsets), advanced with
  real time and nudged ≤ 5% toward that estimate (snaps if > 10 ticks off). Using the estimated *server
  current* tick instead was wrong (it's RTT/2 ahead of any snapshot we can have → 100% starved).
- Delay: adaptive by default → target 1.5 + arrival spread (ticks); grows ≤ 25% of elapsed time, shrinks
  ≤ 5%; starving sets a floor (delay + shortfall + 0.5) that decays 1 tick/s. Fixed delay available
  (`adaptive = false`). Extrapolation off by default (`max_extrapolation_ticks = 0` → hold position).
- Blend origin linearly, angles the short way round; no blending across teleports (> 128u) or alive flips.
- Smoothness thresholds (seed for PLAN 5.x), per 1 ms render frame while the target walks at 320 u/s:
  clean links (50 ms, 50±10 ms): max step < 0.7u and < 0.1% starved — measured 0.336u, 0.00%.
  100±20 ms + 2% loss: max step < 1.6u (≤ 5 ms catch-up), < 0.1% starved — measured 1.04u, 0.03% (one 3 ms hold).

## D27 — Lag compensation (2026-10-22)
- Protocol v2: every command carries the client's view time = snapshot-clock − interpolation delay at the
  moment the command was made (ticks × 256, u32::MAX = none).
- Server keeps a 128-tick rewind buffer of living players' hitboxes, labelled with snapshot tick numbers.
  `Game::step_with(cmds, rewind)` lets the hitscan use a substitute hitbox per (shooter, target);
  plain `step` = no substitution (offline/determinism unchanged; golden hash unchanged).
- anti_lag: target box interpolated between the two recorded ticks around the shooter's view time,
  clamped to [now − max_rewind_ms, now] (default 200 ms). no_anti_lag: current boxes.
- Only hit tests are rewound; damage/knockback apply to the live player state.
- Measured (MemNet, perfect on-screen aim at a target strafing A/D every 400 ms at 300u):
  100 ms RTT → anti-lag 100.0% (mean rewind 119 ms), no-anti-lag 24.7%; 100±20 ms RTT → anti-lag 100.0%;
  600 ms RTT → all rewinds clamped at 200 ms, 12.6%.

## D28 — Network bots & desync metric (2026-10-22)
- `lgduel_bot::netbot::NetBot`: NetClient + Bot + a mirror Game rebuilt from the newest snapshot + Predictor.
  The bot decides on the newest snapshot and reports that tick as its anti-lag view time.
- `lgduel-bot --server H:P [--pattern --skill --name --seed --duration-secs]`, prints a `BOTRESULT` line.
- Desync = prediction error on snapshots *not* preceded by an external change to the predicted player
  (damage taken → knockback, death, respawn). Those are counted separately as `external` corrections;
  detected from authoritative state (damage_taken/deaths/alive), not from events (events can be lost, KI-10).

## D29 — Headless match test (2026-10-22)
- `lgduel-tools match` (Rust, cross-platform) spawns the real `lgduel-server` (--stats-file) and two
  `lgduel-bot` processes (hard vs medium) through an in-process UDP netem proxy, one profile per thread:
  clean (0 ms), lossy (30±10 ms each way, 2% loss each way), highping (100±5 ms each way).
- Asserts: both bots connect (bot + server logs), no non-zero exits / panics, BOTRESULT present, server
  stats file parses with 2 players, damage dealt > 0 for both, lifesteal_earned == dealt × pct / 100 exactly,
  healed ≤ earned, prediction ≥ 100 checks with mean error < 0.5u (desync threshold).
- check.sh runs 60 s (env MATCH_SECS overrides for quick local runs); the three profiles run in parallel,
  so the whole check takes ~70 s.
- `lgduel-tools netem` exposes the same proxy for manual testing.

## D30 — Client online mode (2026-10-22)
- `scene::FrameView` (own eye, remote players, HUD, events) is what the renderer/HUD/audio consume; offline
  `LocalSession::frame_view` and online `NetSession` both produce it, so offline and online share all
  drawing code. `Session` enum in the app (Local + optional bot | Net).
- `NetSession`: NetClient + Predictor (own eye, smoothed) + Interpolator (others, adaptive delay); commands
  carry view time = snapshot clock − interp delay; HUD from the newest snapshot.
- `lgduel --connect HOST:PORT [--name N]` (windowed); `lgduel --headless --connect … --secs N` = online
  smoke check (renders the remote opponent offscreen, asserts forced colours). check.sh stage via
  `lgduel-tools client-smoke`.

## D31 — Netem matrix thresholds (2026-10-22)
Matrix: one-way latency {0, 50, 100, 200 ms} × loss {0, 1, 2.5, 5 %} per direction × jitter ±20 ms (0 ms row
without jitter), 12 s measured after 3 s warm-up, MemNet. Asserted per cell:
- own-movement prediction mean < 0.5u with ≥ 500 checks (desync);
- missing (late/lost) commands ≤ 0.5% of client-ticks;
- remote-player interpolation starvation ≤ 1% of frames;
- largest remote jump per 1 ms frame ≤ 0.7u on clean links (0 ms, 0%), ≤ 2.56u (one tick of walking) otherwise.
Measured (it. 34): prediction exact in 15/16 cells (0 ms + 5% loss: mean 0.0023u); missing ≤ 0.033%; starved
≤ 0.31% (only the 0 ms rows with loss and 100 ms/5% exceed 0.02%); max jump 0.32u in 10 cells, ≤ 2.56u worst.
- Fix found by the matrix: spread estimate now includes late out-of-order snapshot arrivals (the ones the
  buffer must cover). Interpolation test went to 0.00% starved / 0.320u in all 3 of its conditions.

## D32 — Hit-reg agreement & the rewind cap (2026-10-22)
- `net::hitreg::HitReg`: per fired command the client records its on-screen verdict (hitscan from the
  predicted eye vs. remote players at their interpolated positions); compared with the server's `Fire`
  event for that tick (snapshots every tick ⇒ events of tick `snap.tick − 1`). Outcomes: agree / false hit
  ("I hit on my screen") / false miss. Real client tracks it (`NetSession.hitreg`), printed in the online smoke.
- Measured (MemNet, ±4° noisy perfect-tracking aim at an A/D strafer, 300u):
  RTT 50 ms: anti-lag 100.0% vs no-anti-lag 54.0%; RTT 100 ms: 100.0% vs 38.2%;
  RTT 150±40 ms + 2% loss: 77.4% with the default 200 ms cap, 100.0% with a 300 ms cap (no-anti-lag 37.0%).
- **Rewind needed ≈ RTT + input-lead margin (2×jitter + 1 tick) + interpolation delay** (the command executes
  `lead` ticks after it was made; the view is snapshot-clock − delay). With the default `max_rewind_ms = 200`
  (spec: "e.g. ≤ 200 ms") full compensation covers RTT up to ~140 ms on a clean link. Kept as default;
  servers for high-ping regions can raise it. Real UDP smoke: 100.0% agreement over 64 shots.

## D33 — Rubber-band detector (2026-10-22)
- Predictor records, per reconcile, the jump between the position shown before and after it, excluding
  corrections caused by being hit (knockback), killed or respawned. `max_correction`, and `rubber_bands` =
  reconciles with a correction > 1u (a visible snap-back).
- Thresholds: netem matrix ≤ 2 per cell (12 s); headless match ≤ 1 per 10 s per bot (lossy + highping runs
  in check.sh). Remote players: matrix max per-frame jump (D31).
- Measured: 0 in 15/16 matrix cells; 0 ms + 5% loss: 2 × 3.3u (input lead is only 1–2 ticks at ~2 ms RTT,
  so a loss burst can make a command late). Headless 60 s matches: see PROGRESS.

## D34 — Net-graph (2026-10-22)
- Online HUD overlay (bottom-right, colour (1, 0.85, 0.4), on by default, F3 toggles, `--no-net-graph`):
  ping ± jitter (RTT and its mean deviation), loss %, server tick rate, measured snapshot rate (last 1 s),
  interpolation delay (ms), in/out KB/s (connection byte counters, 2 s window), own rubber-band count, hit-reg
  agreement %. Headless online smoke asserts plausible values and that the overlay is drawn.

## D35 — Friend-test release (2026-10-22)
- `lgduel host …` runs the dedicated server inside the game exe (shared `lgduel_server::host` module with
  `lgduel-server`), printing ready-to-copy connect commands (127.0.0.1 and the LAN IP via a connected UDP
  socket to TEST-NET-1, no packet sent). One exe is enough for both players.
- `.cargo/config.toml`: `+crt-static` for x86_64-pc-windows-msvc → no VC++ redistributable needed (verified:
  no vcruntime/msvcp/api-ms-win-crt/ucrtbase imports).
- `scripts/package.sh` → `dist/LGDuel-<os>-x64.zip` after smoke-testing the packaged binaries.

## D36 — Robustness under process stalls (2026-10-22)
- Clock snaps (> 250 ms error) never re-issue a tick; after a forward jump the client still generates
  commands for up to 64 skipped ticks. Predictor replays commands in strictly increasing tick order.
  (Before: a backward snap re-issued ticks with different content → server kept the first → guaranteed
  misprediction; seen as an 88u correction under CPU load.)
- Late-command feedback: a newest snapshot of tick T whose last_cmd < T−1 while we sent a command for T−1
  means ours was late/lost → +1 tick extra input lead (cap 8), decaying 0.25 tick/s.
- Server stats file records `missing_cmds` per player; the match harness stops the netem proxy only after the
  server exits (in-flight Disconnects were being dropped, inflating missing counts by ~333).
- Bots measure prediction/rubber-bands/late commands after a 5 s warm-up. Root cause of intermittent
  check failures: right after a rebuild, freshly started processes stall in their first seconds (consistent
  with antivirus scanning new binaries) → bursts of late commands. After warm-up: 0 late / 0 rubber-bands in
  two consecutive fresh-build checks; under 100% CPU load on all cores: ≤ 5 small (5–10u) corrections per
  bot per 60 s, matching ≤ 8 late commands.

## D37 — Publishing (2026-10-22)
- Public repo https://github.com/s0pk/lgduel (user's choice: public, name `lgduel`). Commit author/committer
  email rewritten (before first push, user-approved) to the GitHub noreply address
  133751159+s0pk@users.noreply.github.com; repo-local git config uses it for future commits.
- Releases: tag `vX.Y.Z`, asset `LGDuel-windows-x64.zip` from `scripts/package.sh` (smoke-tested before upload,
  and the downloaded asset smoke-tested after). v0.1.0 = first playable test build.
- `gh` CLI installed via winget (user-approved); user logged in themselves.

## D38 — v0.2 direction (user, 2026-10-22)
- Public play via a **dedicated server on a rented Linux VPS** (user rents it; we provide Docker/systemd + DEPLOY.md).
- **Launcher = main menu inside lgduel.exe**; server list = shipped `servers.toml` + manual IP (no master server).
- **Maps with ramps + platforms** → convex brushes in collision/rendering.
- UI toolkit: **egui 0.37** (verified: resolves to exactly wgpu 30.0.1 / winit 0.30.13 alongside ours).
- Multi-arena server (old Phase 6 core) pulled forward: required for a public server.

## D39 — UI layer (2026-10-22)
- egui 0.37 + egui-wgpu + egui-winit (`client/src/ui.rs`), painted in its own pass after the scene/HUD
  (LoadOp::Load). While a menu is open, window events go to egui first; game keys/clicks are ignored; Esc
  toggles the pause menu (releases/regrabs the cursor). Online sessions keep running behind the menu.
- **Font:** egui's bundled fonts (Ubuntu/OFL licences) are NOT enabled. `client/src/ttf.rs` generates a
  TrueType font in memory from our original 5×7 glyphs (`font.rs`): each lit pixel = a square contour;
  tables cmap(12)/glyf/head/hhea/hmtx/loca/maxp/post; lowercase maps to uppercase; sizes 16/24 px (2×/3× pixel
  scale). Verified by parsing with skrifa (the reader egui uses).
- Headless egui: frames carry an advancing clock (window fade-in) and texture deltas of skipped frames are
  kept (`Ui::skip`) — the font atlas from the first frame was otherwise lost (UI drew nothing).

## D40 — Settings persistence (2026-10-22)
- `client::settings::Settings` (name, mouse, video{fullscreen,size,vsync,fps_cap,fov}, effects, hud+crosshair,
  enemy colours, audio, practice{bot,pattern,skill}, user servers) serialised as compact TOML at
  `%APPDATA%\LGDuel\settings.toml` / `~/.config/lgduel/settings.toml` (XDG) / macOS Application Support.
  Missing keys → defaults, unknown keys ignored, values sanitised; broken file → defaults + warning, file
  left untouched; writes are atomic (tmp + rename). Created with defaults on the first windowed run.
- Command-line flags override settings for that run only (not written back). Headless runs and
  `--no-settings` never touch the file (tests stay hermetic); `--settings PATH` picks another file.

## D41 — Main menu / session lifecycle (2026-10-22)
- `lgduel` with no session-starting flag opens the **main menu** (Practice vs bot, Play online, Settings,
  Quit) over a slow orbit of the arena. `--practice` (and `--synthetic-input`) start practice directly,
  `--connect ADDR` starts connecting directly.
- Online: Connecting screen until the first frame from the server; back to the Online screen with a message
  on reject, 6 s without an answer, or 5 s without snapshots after joining ("Lost connection"). Addresses
  without a port get 27960.
- In game, Esc = pause menu; "Leave practice"/"Disconnect" returns to the main menu (no longer quits).
  On menu screens Esc goes one level back; the cursor is only grabbed while a session runs with no menu.

## D42 — Server browser (2026-10-22)
- List = shipped `config/servers.toml` (embedded; official, not removable; currently "This PC") + player
  entries in settings (Add/Remove in the Online screen). Each entry is pinged with the info query on open,
  every 5 s and on Refresh; shows name (server-reported), ping, players/max, map, Vamp %/Classic, anti-lag,
  Join (disabled for other protocol versions); "…", "no answer" (2 s), "bad address".
- Hostnames resolve on background threads (a slow/unknown DNS name must never freeze the UI; found by a
  test whose ping was inflated by an inline lookup).

## D43 — Hosting from the menu (2026-10-22)
- `lgduel_server::host::run_config(cfg, duration, stats_file, stop, on_bound)`: shared by `lgduel-server`,
  `lgduel host` and the in-game host; `on_bound` reports the bound address or the bind error to the caller.
- Client `HostHandle` runs it on a background thread (dropped = stop flag + join). "Host a game" screen with
  saved `HostSettings` (name, UDP port, vamp/classic, lifesteal %, anti-lag) → start server → auto-join
  127.0.0.1:port; pause menu shows "Hosting: friends connect to <LAN IP>:<port>"; leaving stops the server.
  `--host-game` does the same from the command line.

## D44 — Convex brushes (2026-10-22)
- `sim::brush::ConvexBrush` = normalised face planes + bounds + axial bevel planes (added unless a face already
  has that normal; never drawn). Vertices by triple-plane intersection (f64), faces = vertices on a plane sorted
  around the centroid. `ConvexBrush::ramp(min, max, rise ±x/±y)` builds wedges.
- `World.convex` traced by `trace_convex` (Quake brush trace: plane dist pushed out by the box corner that
  reaches furthest into the plane, same enter/leave/epsilon logic as boxes) and merged with the box trace
  (nearest wins, solid flags OR-ed). Box-only maps skip the convex path entirely → bit-identical behaviour
  (determinism golden hash unchanged).
- pmove needed no changes for slopes (ground normal, MIN_WALK_NORMAL 0.7 ≈ 45.6°, clip velocity).

## D45 — Map agreement: name + geometry hash in Accept (2026-10-22)
- Protocol v3: `Accept` carries `map` (the map's `name`) and `map_hash` = `World::content_hash()`, an FNV hash
  of the *parsed* geometry (boxes, convex planes incl. bevels, spawns; not colours). Hashing parsed values, not
  file bytes, makes CRLF/LF checkouts and comments irrelevant.
- Clients (NetSession, NetBot) switch to the built-in map with that name if its hash matches; otherwise they
  disconnect with a clear message ("differs from yours" / "doesn't have"). Custom-map download is later work.
- Built-in maps: `world::builtin::ALL` (arena_flat, arena_pillars, arena_tiers). Server `[server] map` /
  `--map` takes a built-in name or a file path; client practice/host map in settings + menu, `--map`.
- Renderer re-uploads the world mesh only when the content hash changes (called every frame).

## D46 — Ramp-top hop kept (2026-10-22)
- Running up a ramp onto a flat top leaves the ground briefly (~14.6u hop on arena_pillars' 27° ramps at run
  speed): pmove's Q3-style "kickoff" check (upward velocity into the new flat ground plane → airborne). This is
  authentic Q3 behaviour and stays; tests/map_movement.rs pins it as a small hop (5–20u).
- Walking up a slope keeps 3D speed (Q3 rescale after clipping), so horizontal speed is 320·cos θ (300 u/s on
  arena_tiers' 20.6° ramp). Golden: tiers ground→mid deck 224 ticks, →high deck 677; stairs top at tick 128.

## D47 — Multi-arena server (2026-10-22)
- `Server.arenas: Vec<Arena>`, `max_arenas` (default 8) pre-created at start and stepped together every tick
  (empty ones too), so every arena has the same tick number and a client moving between arenas sees no clock
  jump. Each `Arena` = its own `Game` + anti-lag rewind buffer (isolation by construction, D10).
- Player ids are unique server-wide (`Game::add_player_with_id`), so a client keeps its id when moved.
- Auto-queue (`Server::place`, run on connect, leave and every tick): queued clients (connect order) go to an
  arena with one player, else an empty one; two lone players in different arenas are paired (the later
  arena's player moves). Players beyond 2·max_arenas wait queued and are sent arena 0's snapshots (watching);
  clients not in the snapshot send idle commands so the connection and clock stay alive.
- Stats: a client's stats = stats from arenas it left (`past`) + current arena.

## D48 — Practice bot, spectating, status line (2026-10-22)
- Protocol v4: reliable `Control::Status(ArenaStatus{arena, queue_pos, opponent, opponent_bot, spectating,
  watching})` server → client whenever it changes; `Control::Spectate{on}` client → server.
- Practice bot (`practice_bot`, default on, `practice_bot_after_ms` 2000): an arena with exactly one human for
  that long gets a server-side `lgduel_bot::Bot` (medium aim, random A/D; full game knowledge, but aim error
  from the skill preset). It leaves as soon as the arena has 0 or 2 humans. Bot ids come from the same pool as
  client ids. Bots are excluded from the stats file and the browser's player count.
- Spectators leave their arena (stats kept) and are never queued or placed; spectators and queued players get
  the busiest arena's snapshots. The client renders a watched player first-person (their view angles and beam;
  HUD shows their numbers) and keeps sending idle commands. Esc menu online: Spectate / Play.
- HUD status line (top centre): "ARENA n - VS NAME", "... PRACTICE BOT UNTIL AN OPPONENT JOINS",
  "QUEUE #k - WATCHING ARENA n", "SPECTATING ARENA n - ESC > PLAY TO JOIN".

## D49 — Reliable game events (2026-10-22)
- Every event except `Fire` (beam visuals: only useful fresh) is reliable. The server numbers them per client
  (`event_base` + queue) and repeats the whole unacked queue in every snapshot as `reliable_events` with the
  first sequence number `event_seq`; when the client acks snapshot T, events up to that snapshot's mark are
  dropped. The client delivers each sequence number once (appended to `events`), counting gaps.
- Wire: count, then the first sequence as a varint only when the list is non-empty (idle delta +1 byte).
  Protocol v5. Queue cap 256 per client (oldest dropped and counted; never hit in tests).
- Effect: hit sounds, damage numbers, kill/fight notices survive loss; they may arrive one RTT late when
  the first copy is lost. Resolves KI-10.

## D50 — Challenges and the roster (2026-10-22)
- Protocol v6: `Control::Roster{replace, players ≤ 16}` (server → client on change; chunks keep each reliable
  message small, since one larger than the packet budget would block the reliable queue), `Challenge{target}`,
  `Respond{from, accept}` (client → server); `ArenaStatus.{challenge_from, challenge_to}`. `ServerEvent{text}`
  is now shown on the client for 4 s in the status line ("BOB DECLINED", "CHALLENGE EXPIRED", …).
- One outgoing challenge per client, expiring after 15 s or when the target leaves. Accept → both go to an
  empty arena, else into the arena of whichever of them is alone; their old opponents are re-paired by the
  normal pairing step (or get the practice bot). No arena available → both are told, nobody moves.
- UI: Esc menu (online) lists players with arena/queued/spectating and a Challenge button; an incoming
  challenge shows Accept / Decline there and replaces the HUD status line ("X CHALLENGES YOU - ESC TO ANSWER").

## D51 — Admin console, RCON, live config (2026-10-22)
- `server::admin`: one command set for the terminal console (stdin thread, only for `lgduel-server` /
  `lgduel host`, not the in-game host) and RCON: status, kick, say, get/set `section.key`, reload, help.
- `set` edits the running config as a TOML table and re-parses it with `ServerConfig::from_toml_str`, so it is
  validated exactly like the file; `apply_config` refuses physics / tick rate / map / bind / port /
  max_arenas changes ("need a restart": clients predict with the physics from their accept). Rules, LG and
  server settings apply to all arenas at once. Connected clients keep their copy of the gameplay config
  (only used for visuals and the hit-reg metric) until they reconnect.
- RCON (protocol v7: `Connectionless::{Rcon, RconReply}`), off unless `rcon_password` is set. Wrong password
  → no reply (no amplification); 5 wrong tries per IP → ignored for 60 s. Replies capped at 1100 bytes.
  Plain-text password over UDP, like the classic Quake rcon: use a long random one.

## D52 — LAN discovery (2026-10-22)
- On every browser refresh the client sends discovery info queries to 255.255.255.255 and 127.0.0.1 on
  ports 27960–27963 (`LAN_PORTS`); a discovery nonce stays open for every answer until it times out.
- Protocol v8: `ServerInfo.instance` (keyed hash of the server secret, stable per process) identifies a server
  reached on several addresses — a host PC answers on 127.0.0.1 and on every adapter (seen in testing: the
  broadcast reply came from a 172.31.x Hyper-V/WSL adapter, not the "LAN IP"), so de-duplication by address
  can't work. One row per instance, 127.0.0.1 preferred, none for servers already listed.
- LAN rows are tagged "LAN" and can't be removed. UI strings avoid "…" (not in the pixel font).

## D53 — Join codes: rendezvous, hole punching, relay (2026-10-22)
- `lgduel-server relay` (server/relay.rs, UDP 27950, generic over Transport): hosts `Register` every 5 s and
  get a code (4 chars from a 31-letter alphabet without 0/O/1/I/L, shown "LG-XXXX"); `Lookup` answers the
  client with the host's public address and tells the host (`Incoming`) so both send `Punch` datagrams and
  open their NATs; `RelayOpen` gives a session and the service forwards `RelayData` between exactly that
  client and host (256 KB/s per direction cap). Stranger-sendable requests are padded to ≥ any answer;
  lookups/relay requests limited to 30 per IP per minute; hosts expire after 30 s, sessions after 30 s idle.
- `net::rendezvous::RelayTransport` wraps the game socket (same socket = same NAT mapping): host mode
  registers and maps relayed clients to virtual addresses `[fd00:4c47:0:1::session]:1`; join mode always
  presents the host as `JOIN_ADDR` and routes direct (after `Found`) or through the relay (no answer from
  the host within 2.5 s, or `force_relay`). Server and NetClient code are unchanged above it.
- MemNet NAT simulation (port-restricted cone, symmetric) tests both paths; check.sh `p2p-smoke` runs the real
  binaries on 127.0.0.1 (bot direct, client forced through the relay). Settings: `rendezvous` (player) or the
  shipped `config/servers.toml` `rendezvous` (empty until the user's VPS exists); Host screen "Join code".

## D54 — Master list in the relay service; browser filters (2026-10-23)
- The rendezvous service is also the master server: `Register{listed: true}` (server setting `public`, needs
  `rendezvous`) puts the host in the public list; it drops out 30 s after its last registration.
- `ListRequest` → `ListChallenge{cookie}` (keyed hash of the requester's address) → `ListRequest{cookie}` →
  `List` (≤ 100 addresses): the only big answer goes to a proven address. Loopback entries (a server on the
  relay's own VPS registering via 127.0.0.1) are rewritten by the browser to the master's address.
- Protocol v9: `ServerInfo.region` (≤ 12 chars, server setting `region`). Browser filters (saved in settings):
  mode, anti-lag, max ping, region substring, hide empty / full; rows that don't answer are always shown;
  "N hidden by the filters" line. Public rows tagged PUBLIC, deduped with listed/LAN rows by instance.

## D55 — Console, cvars, profiles (2026-10-23)
- Cvars are the settings file's own keys (dotted paths of `Settings` serialised to TOML, any depth; the
  player's server list is not a cvar), so there is one source of truth and no separate cvar registry to
  keep in sync. `set` edits the TOML table and re-parses through `Settings::from_toml` (sanitise), refusing
  unknown keys and wrong value types; "KEY VALUE" works like Quake; a few aliases (sensitivity, dpi, fov,
  volume, beam, name).
- Profiles: `writeconfig NAME` writes `set KEY VALUE` lines to NAME.cfg next to settings.toml; `exec NAME`
  runs a file (comments // and #, nesting ≤ 4). Names restricted to [A-Za-z0-9_-] (no paths).
- UI: ` toggles an opaque top console (egui Area; egui 0.37 panels need a parent Ui); it counts as an open
  menu (mouse released). Console lines can connect/join/practice/disconnect/quit.

## D56 — Input display, gun model, no shake / no blood (2026-10-23)
- Input display (HUD, off by default): W / A S D / JP CR LG keys above the bottom centre, lit when held; own
  keys while playing, the watched player's jump/crouch/fire while spectating (movement keys aren't in
  snapshots).
- Gun model `effects.gun = none | small | normal` (default none): two flat-shaded boxes along the muzzle line
  in view space, not drawn while dead.
- `scene::view_camera` is the only camera source; tests assert the camera and the whole dynamic scene are
  identical with and without damage/kill events (no screen shake, kick, blood, gibs or flashes).
- With these, every §1 visibility option is in Settings → Visuals (PLAN 8.2).

## D57 — Demos (2026-10-23)
- `net::demo` (`.lgdemo`, "LGDM" v1): header (kind, map + content hash, gameplay config TOML, recording
  player, initial state) + frames. Local demos: initial full snapshot + every tick's commands of every player
  + End{tick, state_hash}; playback re-simulates with `LocalSession::step_with_cmds` and reports Match /
  Mismatch / Unverified. Net demos: snapshots as received (full encoding, arrival time) + own commands;
  playback shows the snapshot at the playback clock (no re-simulation: the server's state is the truth).
- Playback view = the recorder's own angles (SpectateView) with the input overlay forced on; status line
  says whether the replay matched. Console record/stoprecord/playdemo (demos/NAME.lgdemo next to the
  settings), CLI --record/--play-demo; headless record (scripted practice duel vs the bot) + verified replay
  in check.sh.

## D58 — Release profile and docs (2026-10-23)
- `[profile.dist]` (fat LTO, 1 codegen unit, stripped) for release packages: lgduel.exe 15.0 → 13.4 MB, same
  frame times; check.sh keeps release-dbg (dist takes ~2 min). Measurements in docs/PERF.md.
- Docs split by audience: README (overview), PLAY (players), HOSTING (all ways to host), DEPLOY (VPS),
  CONFIG (every server and client key; `lgduel --print-settings`), MAPS (format, movement rules, limits).
  Packages ship all of them.

## D59 — Map material tags (2026-10-23)
- Optional `material` on brush/ramp/stairs/convex entries → `World.materials` (parallel to `brushes`, stairs steps
  share their entry's tag) and `ConvexBrush.material`. Purely visual: not in `content_hash`, not used by sim/trace.

## D60 — Textured world pipeline (2026-10-23)
- World geometry draws with its own pipeline (`WorldVertex` = pos, uv, tint, layer; 2D texture array, 256², 9 mips,
  repeat/linear/trilinear). Dynamic geometry and HUD keep the flat pipeline. Layer 0 is white: tint × white = the
  old flat colour, so `ClassicStyle` reproduces the previous output exactly (test compares vertex by vertex with the
  old `world_mesh`, kept test-only).
- `WorldStyle` trait (`key`, `textures`, `look(map, Surface)`) is what themes implement; the renderer re-uploads when
  (map content hash, style key) changes. UVs are planar along the dominant normal axis.

## D61 — Themes (2026-10-23)
- `theme.rs`: `Theme` (sky, defaults by orientation, materials, per-map overrides; BTreeMaps so the style key is
  stable) → `ThemeStyle` (implements `WorldStyle`: textures loaded into layers, key = fingerprint). Resolution:
  map tag (if the theme defines it) → orientation default (floor n.z>0.7, ceiling < -0.7, wall, ramp = convex face
  with 0.2<n.z≤0.99) → classic look. Built-ins: `classic` (empty theme = the old look, sky 0.02/0.02/0.03) and `grid`
  (procedural tiles/concrete/panel, light greys tinted by the map colour). Setting `video.theme`, `--theme`,
  Visuals → Map → Theme; the App rebuilds the style when the setting changes (also via the console cvar).
- Headless checks: classic keeps the exact floor-colour check; other themes require ≥ 50 distinct colours and the
  same enemy/HUD/beam checks.

## D62 — User themes from folders (2026-10-23)
- `ThemeStyle::load(name, dirs)`: built-in names first, else `<dir>/<name>/theme.toml` from `--theme-dir`,
  `<settings folder>/themes`, `<exe folder>/themes`. The folder name is the theme's identity (`requested` is what the
  App compares, so a failed load doesn't rebuild every frame). Every failure → classic look + a problem message
  (terminal + console); a bad texture only affects its own material.
- Limits: PNG only, ≤ 2048², ≤ 4 MB, ≤ 32 textures, relative names without `..` / `:` / `\` (same answer on every
  OS), scaled to 256². Style key hashes all pixels so edited files re-upload on `reloadtheme`.
- `lgduel --make-example-theme DIR` writes the example (CC0); a test fails if `assets/themes/example` is stale.
  Packages ship it under `themes/example`.

## D63 — Enemy models and skins (2026-10-23)
- `models.rs`: `ModelKind::{Classic, Humanoid}`, `ModelSettings {enemy, forced_colors (default true), skin}`.
  Classic = the old `players::push_player` byte for byte. Humanoid = legs (swing from position phase, ±0.28 rad),
  leaning torso, head, two arms + LG aimed by pitch (≤ 0.6 rad), built with the shared `render::push_obox`.
- Everything stays inside a 15-unit-radius cylinder and the hull's height (test over 10 000+ vertices at many yaws,
  pitches, positions, standing/crouched), so it is within the axis-aligned hitbox at every yaw.
- Forced colours are applied per part, not by height band as first sketched: legs → `lower`, torso/arms/gun/→ `upper`,
  head → `head`, unshaded. Skins (forced colours off) are shaded; defaults are warm/dark tones that stand out
  from grey arenas (a first grey-blue default vanished against the floor).
- Legs animate from position only (no velocity in `OtherView`), so a standing player holds a frozen stride.
- Headless: `--enemy-model`, `--skins`, `--enemy-distance` (60–1500) for close-up pictures; the per-band pixel
  minimum is 20 for non-classic models.

## D64 — Linux, share codes, look & feel (2026-10-23)
- GL backend (wgpu) rejects a texture array with one layer: the world texture set always allocates at least 2 layers.
- Profile codes `LGP1-` = zlib + CRC32 + base64url over a whitelisted subset of settings (`profile::is_look_key`) plus an optional embedded theme; hard size caps on decode. Local only: no server, no accounts.
- Theme packs `.lgtheme` unpack with the same path-safety and size limits as theme folders.
- MSAA is a start-up setting (renderer pipelines and targets are made once); default 1 keeps the classic look pixel-identical. `render::Targets` bundles the depth buffer and the multisampled colour buffer that resolves into the real target.
- The scoreboard sits high on the screen (not centred) so the crosshair and enemy stay visible while TAB is held; the existing end-of-session report on leaving covers the "session summary".
- Arch: AUR source package only (split lgduel / lgduel-server); submission to the AUR needs the user's account.

## D65 — KovaaK's import (2026-10-23)
- Copy what we import: converted themes become normal LGDuel theme folders `<settings dir>/themes/kovaaks-<slug>/` (marker file `.kovaaks-import`; a player's own folder of the same name is never overwritten, we use `-2`, `-3`). The KovaaK's folder is only ever read, the `.pak` never.
- Approximate mapping: colours are exact (KovaaK's tints are linear, `skyColor` is sRGB bytes → `srgb8_to_linear`); texture style is the nearest of tiles/concrete/panel/none (`kovaaks::texture_for`); `shade = true` stays on so walls remain distinguishable. A non-solid sky (skybox preset) becomes the same colour with a 0.6× top gradient. Enemy colours are imported with a contrast warning when within 0.25 of the floor or wall tint.
- Theme JSON is read through `serde_json::Value` field by field (not a derive struct): a wrongly typed value loses only that field, and unknown keys are ignored.
- Texture scale: `scale = 128 / kv_scale` (higher KovaaK's scale = more repeats). Assumption, not yet compared against KovaaK's itself; flip in `kovaaks::scale_for` if inverted.
- `kovaaks.path` is a plain `String` (empty = automatic) so it works as a console variable.
- Linux: Steam roots are `$XDG_DATA_HOME/Steam`, `~/.local/share/Steam`, `~/.steam/steam`, `~/.steam/root` and the Flatpak path, deduplicated by canonical path (`~/.steam/steam` is a symlink to the first). Verified against a real Proton install on CachyOS (113 themes, 441 sounds).
- Headless runs now use `--settings FILE` when given (and `opts.s.enemy_colors` for the pixel checks); without it nothing changes, so every existing check stays identical.
- K3/K4: `lewton` (pure-Rust Ogg Vorbis) built fine on the pinned toolchain, no `symphonia` needed. WAV is parsed by hand (tags 1, 3, 0xFFFE; ADPCM refused with a message). Only the first 2 s are kept (20 ms fade when cut); at most 4 hit voices play at once (oldest dropped).
- Custom sounds live in `audio.hit_sound` / `audio.kill_sound_file` (file names in `<settings dir>/sounds/`, `Option`, so they are not console variables; the `hitsound` command sets the hit one). Both are excluded from share codes (`profile::is_look_key`) and sanitized to plain file names.
- K5: one entry point, `kovaaks::run(settings, dir, Cmd)`, serves the console, the Share tab and tests; the CLI flags (`--import-kovaaks-*`) call the library directly so their output lines stay stable for `check.sh`. The Share tab scans the install once (when first shown / "Search again"), not per frame.
- A headless run now loads an explicit `--settings FILE` (otherwise it still ignores settings), which is how `check.sh` renders the imported theme with its enemy colours.
- K6 (crosshair image) was not done (optional).

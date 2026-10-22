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

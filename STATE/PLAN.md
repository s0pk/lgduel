# LGDuel PLAN

Rules: one item per iteration. Pick the highest unchecked item whose deps are done.
Split items that are too big before starting them. `[blocked]` = see KNOWN_ISSUES.md.

## Phase 1 — Skeleton
- [x] 1.1 Cargo workspace: sim, net, server, client (bin `lgduel`), bot, tools; pinned toolchain
- [x] 1.2 `scripts/check.sh`: fmt, clippy -D warnings, release-dbg build, tests, binaries run, explicit SKIP for future stages
- [x] 1.3 STATE files, .gitignore, initial commit
- [x] 1.4 Shared `sim::config` (serde + TOML) with every gameplay default from DECISIONS.md, load/round-trip test

## Phase 2 — Sim core (deps: 1.x)
- [x] 2.1 Math foundation: Vec3, deterministic float policy (no FMA-dependent ops, fixed op order), state hash
- [x] 2.2 Fixed-tick loop abstraction (accumulator, tick index; no clocks inside sim)
- [x] 2.3 Usercmd: view angles (exact, quantized like wire format), forward/right/up move, buttons (jump/crouch/fire)
- [x] 2.4 World geometry: AABB brush list + map data file format, loader + tests
- [x] 2.5 Trace: swept AABB vs brushes (fraction, normal, startsolid), unit tests
- [x] 2.6 pmove ground: friction, accel (Q3-style accelerate), stop speed, ground check
- [x] 2.7 pmove air: air accel, gravity, jump (+ hold-jump behaviour per DECISIONS)
- [x] 2.8 pmove slide-move / clip velocity / step-up 18u, crouch hull + speed scale
- [x] 2.9 Golden tests: standing jump apex, ground accel-to-320 time, strafe-jump speed gain, circle-jump
- [x] 2.10 Strafe-aim relative-speed ratio test: mirroring ≈0.00v, anti-mirroring ≈2.00v, HSW mirroring ≈0.29v, HSW anti ≈1.71v
- [x] 2.11 Hitscan LG: 50 ms refire, range 768, ray vs player AABB, damage, knockback
- [x] 2.12 Damage/health/armor(off)/death/respawn timer, lifesteal % (0–100), exact-heal test
- [x] 2.13 Rules: vamp duel (default), first-to-N classic, fight lifecycle, stats counters
- [x] 2.14 Determinism test: input log replayed twice → identical state hash; wired into check.sh

## Phase 3 — Local playable (deps: 2.x)
- [x] 3.1 Client window (winit + wgpu), low-latency present mode, vsync off default, frame-time p50/p99 logging
- [x] 3.2 Offscreen/headless render path + screenshot to artifacts/ + non-blank/colour assertion in check.sh
- [x] 3.3 Flat-shaded fullbright arena rendering from map file
- [x] 3.4 Raw mouse input, sub-pixel accumulation, cm/360 + DPI sensitivity, FOV 10–130
- [x] 3.5 Local movement through sim at fixed tick, render interpolation of own view
- [x] 3.6 Player models: simple original geometry, forced enemy colours head/upper/lower
- [x] 3.7a LG beam (styles incl. none, own beam from muzzle, enemy beams), impact sparks on/off + size
- [x] 3.7b Hit sound (procedurally generated, original), audio output, volume
- [x] 3.8 HUD: text rendering, health, crosshair, fight stats, damage numbers
- [x] 3.9 Bot crate: strafe patterns (random A/D w/ reaction model, mirror, anti-mirror, HSW), tracking aim w/ accuracy + reaction delay
- [x] 3.10 Offline vs bot practice mode playable

## Phase 4 — Networking (deps: 2.x; 3.x only for client integration)
- [x] 4.1 Transport abstraction + UDP impl; reliable + unreliable channels
- [x] 4.2 Protocol messages: connect/handshake, usercmd packets (seq, redundant last N cmds), snapshots
- [x] 4.3 Snapshot delta compression vs acked baseline
- [x] 4.4 Server binary: fixed tick, config TOML, accepts clients, runs sim
- [x] 4.5 Clock sync + client tick alignment
- [x] 4.6 Client prediction + reconciliation (own movement + own LG beam)
- [x] 4.7 Entity interpolation with configurable buffer; extrapolation off by default
- [x] 4.8 Lag compensation: rewind buffer, anti-lag mode (≤200 ms) and no-anti-lag mode
- [x] 4.9 Bot as network client
- [x] 4.10 Headless match test in check.sh: server + 2 bots, 60 s vamp duel, assertions (connect, no panic/desync, damage, exact lifesteal, stats file)
- [x] 4.11 Prediction error test (<0.5u at 0% loss) in check.sh
- [x] 4.12 Real client connects to 127.0.0.1

## Phase 5 — Netcode hardening (deps: 4.x)
- [x] 5.1 Netem layer in tools (latency, loss, jitter, reorder) usable in-process
- [x] 5.2 Matrix tests: 0/50/100/200 ms × 0–5% loss × ±20 ms jitter; thresholds documented
- [x] 5.3 Hit-reg agreement metric (client-predicted hit vs server hit), anti-lag on/off
- [x] 5.4 Rubber-band detector; one lossy + one high-ping run in check.sh
- [x] 5.5 Net-graph: ping, loss, jitter, tick rate, snapshot rate
- [ ] 5.6 Game events survive snapshot loss: resend events until a snapshot carrying them is acked (dedupe client-side by tick)

## Phase 6 — Multi-duel server (deps: 4.x)
- [ ] 6.1 Arena instances / pair isolation (no cross-pair damage or collision)
- [ ] 6.2 Queue + challenge/accept flow
- [ ] 6.3 Spectate (follow a pair)
- [ ] 6.4 Per-player stats: LG acc %, dmg dealt/taken, W/L, fight length; end-of-fight summary; stats file
- [ ] 6.5 Admin console + RCON; hot-reload of gameplay config values

## Phase 7 — Hosting (deps: 4.x, 6.x)
- [ ] 7.1 `lgduel host [--config x.toml]` subcommand
- [ ] 7.2 In-game Host (listen server)
- [ ] 7.3 LAN discovery
- [ ] 7.4 Rendezvous binary: join codes (LG-XXXX), UDP hole punching
- [ ] 7.5 Relay fallback (same binary); P2P test incl. forced-relay in check.sh
- [ ] 7.6 Master server + server browser with filters (anti-lag, vamp %, region, ping, players)
- [ ] 7.7 Dockerfile + docker-compose.yml
- [ ] 7.8 Release builds for Windows/Linux/macOS (CI notes / cross-check); Linux check.sh run (KI-1)

## Phase 8 — Visibility & settings (deps: 3.x)
- [ ] 8.1 Console + cvar system, `exec`/`writeconfig` profiles
- [ ] 8.2 Settings UI covering every §1 visibility option
- [ ] 8.3 Crosshair editor
- [ ] 8.4 Input display (WASD/jump/crouch)
- [ ] 8.5 Gun model options (configurable/absent), no screen shake, no blood (assert)

## Phase 9 — Demos & stats (deps: 4.x)
- [ ] 9.1 Demo recording (inputs + snapshots)
- [ ] 9.2 Deterministic playback + input overlay; record→playback hash test
- [ ] 9.3 End-of-fight summary UI

## Phase 10 — Perf (deps: 3.x, 4.x)
- [ ] 10.1 Profile client; reach 500+ FPS target at 1080p (document hardware)
- [ ] 10.2 Latency notes, binary size, startup time

## Phase 11 — Packaging & docs
- [ ] 11.1 README quick start (≤5 steps to duel a friend)
- [ ] 11.2 HOSTING.md, CONFIG.md
- [ ] 11.3 Second map (pillars/height variant) + map format doc
- [ ] 11.4 Definition-of-done run, write STATE/DONE

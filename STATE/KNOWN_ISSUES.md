# KNOWN ISSUES

- KI-1 (2026-10-22, RESOLVED 2026-10-23: check.sh green in WSL2 Arch): Dev machine is Windows; DoD requires check.sh green on Linux. No Linux run yet.
  Add a Linux verification path (WSL / Docker / CI) by PLAN 7.8 at the latest.
- KI-2 (2026-10-22, RESOLVED iteration 60: real p2p-smoke stage): check.sh stage p2p prints `SKIP`
  until Phase 7 lands. Absent, not passing. (Offscreen render live since it. 14, headless match since it. 31, netem matrix since it. 34.)
- KI-3 (2026-10-22): cargo is installed at ~/.cargo/bin but may not be on PATH in fresh shells
  until a new login session. Prefix `export PATH="$HOME/.cargo/bin:$PATH"` if `cargo` is not found.
- KI-4 (2026-10-22): Physics constants not yet verified against Quake Live server cvars (pmove_StepHeight
  possibly 22, pmove_JumpVelocity 275, LG damage/knockback). Verify from public docs before Phase 2 ends.
- KI-5 (2026-10-22): pmove has no "all solid" recovery (Q3 jitters the origin to escape). Spawns are
  validated free; revisit if players can get embedded (e.g. crouch/stand edge cases, knockback).
- KI-6 (2026-10-22): No player-player collision in pmove. Quake duelists block each other; add solid
  player boxes (same-pair only) before Phase 6 pair isolation.
- KI-7 (2026-10-22): Offscreen render check needs a GPU adapter. It falls back to wgpu's software adapter,
  but a Linux CI box needs a Vulkan software driver (lavapipe/mesa) installed — document in 7.8.
- KI-8 (2026-10-22): Fullbright flat shading gives few depth cues on large flat floors. Consider a subtle
  floor grid / edge outlines as a visibility option in Phase 8.
- KI-9 (2026-10-22): Audible output of the hit sound is verified only to the point of "stream opened and
  samples submitted" (`lgduel --sound-test` on Realtek @ 48 kHz); a human must confirm it sounds right.
  `--sound-test` is not in check.sh because CI boxes have no audio device.
- KI-10 (2026-10-22): Game events (Fire/Damage/Kill/…) ride only in the snapshot that carries them; a lost
  snapshot loses them (hit sounds, damage numbers, kill notices). State (health, stats, alive) is still
  correct via later snapshots. Fix planned as PLAN 5.6.
  RESOLVED 2026-10-22 (iteration 54, M4.3): non-Fire events are repeated until acked (D49).
- KI-11 (2026-10-22): Default anti-lag cap (200 ms) only fully compensates RTT ≲ 140 ms because the rewind
  includes the input lead and interpolation delay (D32). Consider exposing "max ping fully compensated" in
  the server browser, or a 250–300 ms default after community feedback.
- KI-12 (2026-10-22): Bots don't navigate: they strafe/aim where they are, so on multi-level maps
  (arena_tiers) they often fight across levels or without line of sight (hit rate ~7–10% vs ~60% on
  arena_flat). Fine for match/net tests; real practice on those maps wants waypoint-following bots.
- KI-13 (2026-10-22): Spectators and queued players always watch the busiest arena and follow its first living
  player; there is no way yet to pick an arena or switch the followed player.
- KI-14 (2026-10-22): No Linux verification possible right now: GitHub Actions refuses every job ("account is
  locked due to a billing issue", run 39765708214), and Docker Desktop on the dev PC crashes at start
  ("initializing Inference manager … remove …\Docker\run\dockerInference: The file cannot be accessed").
  Both need the user. KI-1 stays open; the v0.2.0 release needs the Linux server tarball (built by CI).

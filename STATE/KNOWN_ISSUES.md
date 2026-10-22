# KNOWN ISSUES

- KI-1 (2026-10-22): Dev machine is Windows; DoD requires check.sh green on Linux. No Linux run yet.
  Add a Linux verification path (WSL / Docker / CI) by PLAN 7.8 at the latest.
- KI-2 (2026-10-22): check.sh stages headless match / netem / p2p / offscreen render print `SKIP`
  until their phases land (4, 5, 7, 3). They are absent, not passing.
- KI-3 (2026-10-22): cargo is installed at ~/.cargo/bin but may not be on PATH in fresh shells
  until a new login session. Prefix `export PATH="$HOME/.cargo/bin:$PATH"` if `cargo` is not found.
- KI-4 (2026-10-22): Physics constants not yet verified against Quake Live server cvars (pmove_StepHeight
  possibly 22, pmove_JumpVelocity 275, LG damage/knockback). Verify from public docs before Phase 2 ends.
- KI-5 (2026-10-22): pmove has no "all solid" recovery (Q3 jitters the origin to escape). Spawns are
  validated free; revisit if players can get embedded (e.g. crouch/stand edge cases, knockback).
- KI-6 (2026-10-22): No player-player collision in pmove. Quake duelists block each other; add solid
  player boxes (same-pair only) before Phase 6 pair isolation.

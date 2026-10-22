# KNOWN ISSUES

- KI-1 (2026-10-22): Dev machine is Windows; DoD requires check.sh green on Linux. No Linux run yet.
  Add a Linux verification path (WSL / Docker / CI) by PLAN 7.8 at the latest.
- KI-2 (2026-10-22): check.sh stages headless match / netem / p2p / offscreen render print `SKIP`
  until their phases land (4, 5, 7, 3). They are absent, not passing.
- KI-3 (2026-10-22): cargo is installed at ~/.cargo/bin but may not be on PATH in fresh shells
  until a new login session. Prefix `export PATH="$HOME/.cargo/bin:$PATH"` if `cargo` is not found.

# PROGRESS (append-only)

## 2026-10-22 — iteration 1: bootstrap (PLAN 1.1–1.3)
- Installed Rust toolchain on the dev machine (Windows 11, MSVC host): rustup via winget, rustc 1.99.1 stable + rustfmt + clippy.
- Created Cargo workspace (edition 2024, resolver 3) with crates: sim, net (libs); server, client (bin `lgduel`), bot, tools (bins). Toolchain pinned in rust-toolchain.toml (1.99.1).
- `scripts/check.sh`: fmt --check, clippy -D warnings (all targets), release-dbg build, workspace tests, runs each binary. Future stages print an explicit `SKIP` naming the phase that adds them (no fake passes).
- Verified: `./scripts/check.sh` → CHECK GREEN (Windows, Git Bash). Tests: 2 passed (sim tick = 8000 µs at 125 Hz; protocol version nonzero).
- Measured: n/a (no runtime code yet).

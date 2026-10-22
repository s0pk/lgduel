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

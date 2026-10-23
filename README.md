# LGDuel

A standalone **Lightning-Gun duel trainer** in the style of the classic arena-shooter LG duel scene
(the "x76" vamp-duel format): two players, LG only, fighting over crosshair uptime vs. dodging.

> Working on the code? Start with [HANDOFF.md](HANDOFF.md) (repo overview, rules, next feature plan).

- **Vamp mode by default** — 100% lifesteal: damage you deal heals you, so fights are decided by the
  accuracy gap. Classic first-to-N duels and any lifesteal % are server settings.
- **Arena-shooter movement** — 125 Hz fixed-tick physics with strafe-jumping, circle-jumps and the
  strafe-aim relative speeds (mirroring 0v, anti-mirroring 2v, half-sideways 0.29v / 1.71v) falling out of the physics.
- **Netcode** — authoritative server, client prediction (bit-exact), smooth interpolation, and both
  server flavours: **anti-lag** (lag-compensated hitscan) and **no-anti-lag**.
- **Visibility first** — fullbright flat-shaded arena, solid forced enemy colours (head / body / legs),
  beam styles incl. none, impact sparks, hit sound, damage numbers, net-graph.
- **Practice bots** that strafe like players (random A/D, mirroring, anti-mirroring, half-sideways) with
  tunable reaction time and aim.
- **Menus and settings in game**, a server browser, hosting from the menu, borderless fullscreen.
- **Maps with height** — ramps, stairs and platforms (`arena_flat`, `arena_pillars`, `arena_tiers`).
- **Dedicated servers** run many duels at once with a queue, a practice bot for lone players and
  spectating; one command installs one on a Linux VPS ([DEPLOY.md](DEPLOY.md)).

Windows build provided; Linux/macOS: build from source.

## Play

1. **Download** `LGDuel-windows-x64.zip` from [Releases](../../releases), unzip it, run `lgduel.exe`.
2. **Practice** duels a bot offline. **Play online** lists servers: click **Join**. **Host a game**
   runs a server on your PC for friends.
3. **Esc** opens the menu (settings, spectate, disconnect). **F11** toggles fullscreen.

Full guide (online play, hosting, all settings, what to report): **[PLAY.md](PLAY.md)**.
More: [HOSTING.md](HOSTING.md) (every way to host) · [DEPLOY.md](DEPLOY.md) (VPS server in one command) ·
[CONFIG.md](CONFIG.md) (all settings) · [MAPS.md](MAPS.md) (map format) · [THEMES.md](THEMES.md) (textures and themes) · [SHARING.md](SHARING.md) (share codes, theme packs) · [KOVAAKS.md](KOVAAKS.md) (use your KovaaK's themes and hit sound) · [INSTALL.md](INSTALL.md) (Arch/Linux) · [docs/PERF.md](docs/PERF.md).

## Controls

Mouse look (raw input) · **WASD** move · **Space** jump (hold to hop) · **Ctrl/C** crouch ·
**Left mouse** fire · **Esc** menu · **F3** net-graph · **F11** / **Alt+Enter** fullscreen.

Command-line options override the settings for one run, e.g. `--sens-cm 30 --dpi 800`, `--fov 110`,
`--practice --map arena_tiers --bot mirror --bot-skill hard`, `--connect HOST:PORT`, `--fullscreen`.

## Running a server

- **On a VPS (recommended for playing over the internet):** see [DEPLOY.md](DEPLOY.md) — open UDP 27960,
  run one install command, add the address in Play online.
- **On your PC:** Host a game in the menu, or `lgduel.exe host [--config server.toml] [--map arena_tiers]`.
  `lgduel-server.exe` is the same server as a separate binary; `lgduel-bot.exe --server HOST:PORT` adds a bot.

Rules live in a config file (copy `server.example.toml`): vamp/classic, `lifesteal_percent`,
`hit_mode = "anti_lag" | "no_anti_lag"`, `max_rewind_ms`, `max_arenas`, `practice_bot`, map, tick rate,
physics, LG damage/range.

## Build from source

Requires Rust (stable, pinned in `rust-toolchain.toml`).

```
cargo build --release
target/release/lgduel host
target/release/lgduel --connect 127.0.0.1:27960
```

`./scripts/check.sh` runs formatting, lints, all tests, a determinism replay, a network-conditions matrix,
three 60-second bot-vs-bot server matches (clean / lossy / high-ping, one per map), online client smoke
tests (playing and spectating) and offscreen render checks. CI runs it on Linux with a software Vulkan
driver (`.github/workflows/ci.yml`); `./scripts/linux-check.sh` does the same locally in Docker.
`./scripts/package.sh` builds the release zip; pushing a `v*` tag publishes a release.

## Project layout

| crate | |
|---|---|
| `sim` | deterministic simulation: movement, LG hitscan, damage/lifesteal, rules |
| `net` | protocol, delta snapshots, prediction, interpolation, lag compensation |
| `server` | dedicated server (`lgduel-server`, `lgduel host`) |
| `client` | renderer, input, HUD, audio (`lgduel`) |
| `bot` | practice / test bots (`lgduel-bot`) |
| `tools` | netem proxy, headless match harness |

## License

MIT — see [LICENSE](LICENSE). All code and assets are original; no game assets from other titles are used.
Names of other games are used only descriptively.

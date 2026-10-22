# LGDuel

A standalone **Lightning-Gun duel trainer** in the style of the classic arena-shooter LG duel scene
(the "x76" vamp-duel format): two players, LG only, fighting over crosshair uptime vs. dodging.

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

> Early test build: no menus yet — settings are command-line options. Windows build provided; Linux/macOS
> build from source.

## Play with a friend in 3 steps

1. **Download** `LGDuel-windows-x64.zip` from [Releases](../../releases) and unzip it (both players).
2. **One player hosts:** open a terminal in the folder and run

   ```
   lgduel.exe host
   ```

   It prints the command the friend should use. Allow it through the Windows firewall if asked.
3. **Both connect** (the host in a second terminal):

   ```
   lgduel.exe --connect 127.0.0.1:27960 --name host          # host
   lgduel.exe --connect <host address>:27960 --name friend   # friend
   ```

   `<host address>` is the host's LAN IP (same network), Tailscale/ZeroTier IP (easiest over the
   internet), or public IP with **UDP port 27960 forwarded** on the host's router.

Full guide (connection options, all settings, what to report): **[PLAY.md](PLAY.md)**.

## Practice offline

```
lgduel.exe                                   # duel a bot (random A/D strafer, medium aim)
lgduel.exe --bot mirror --bot-skill hard     # patterns: still ad mirror anti hsw hswanti
                                             # skills:   easy medium hard aimbot
```

## Controls

Mouse look (raw input; click to capture, **Esc** to release) · **WASD** move · **Space** jump (hold to hop) ·
**Ctrl/C** crouch · **Left mouse** fire · **F3** net-graph.

Common options: `--sens-cm 30 --dpi 800` (sensitivity as cm/360), `--fov 110`, `--size 1920x1080`,
`--beam none|thin|thick|zigzag`, `--no-sparks`, `--volume 0.5`, `--vsync`, `--fps-cap 240`.

## Hosting options

`lgduel.exe host --port N --config server.toml` — copy `server.example.toml` and edit rules: vamp/classic,
`lifesteal_percent`, `hit_mode = "anti_lag" | "no_anti_lag"`, `max_rewind_ms`, tick rate, physics, LG damage/range.
`lgduel-server.exe` is the same server as a separate binary; `lgduel-bot.exe --server HOST:PORT` adds a bot to a server.

## Build from source

Requires Rust (stable, pinned in `rust-toolchain.toml`).

```
cargo build --release
target/release/lgduel host
target/release/lgduel --connect 127.0.0.1:27960
```

`./scripts/check.sh` runs formatting, lints, all tests, a determinism replay, a network-conditions matrix,
three 60-second bot-vs-bot server matches (clean / lossy / high-ping) and offscreen render checks.
`./scripts/package.sh` builds the release zip.

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

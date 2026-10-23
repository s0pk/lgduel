# Configuration reference

## Server (`server.toml`)

Copy `server.example.toml` (it lists every key with its default and a comment) and pass it with
`lgduel-server --config server.toml`. Every key is optional; unknown keys are an error (typos don't go
unnoticed). While the server runs, `get` / `set section.key value` / `reload` in its console or over RCON
change rules, LG and `[server]` settings live; physics, tick rate, map, bind/port, rendezvous, public and
`max_arenas` need a restart.

### `[server]`

| Key | Default | |
|---|---|---|
| `name` | "LGDuel server" | shown in the browser (≤ 32 chars) |
| `bind`, `port` | "0.0.0.0", 27960 | UDP address |
| `max_clients` | 16 | players + spectators + queue |
| `max_arenas` | 8 | separate duels (2 players each) |
| `map` | "arena_flat" | built-in name or a map file ([MAPS.md](MAPS.md)) |
| `snapshot_every` | 1 | snapshot every N ticks (1 = 125/s) |
| `timeout_ms` | 5000 | drop silent clients |
| `practice_bot`, `practice_bot_after_ms` | true, 2000 | a lone player gets a bot after this long |
| `rcon_password` | "" | remote console (off when empty) |
| `rendezvous` | "" | join-code / public-list service, `host[:port]` |
| `public`, `region` | false, "" | list publicly (needs `rendezvous`); region label |

### Gameplay

| Section | Keys |
|---|---|
| top level | `tick_rate_hz` (125) |
| `[physics]` | `gravity` 800, `max_speed` 320, `jump_velocity` 275, `accelerate` 10, `air_accelerate` 1, `friction` 6, `stop_speed` 100, `step_height` 18, `crouch_speed_scale` 0.25, `player_mins`/`player_maxs` (30×30×56 box), `crouch_maxs_z` 16, `view_height` 26, `crouch_view_height` 12, `auto_hop` true, `snap_velocity` true |
| `[lg]` | `refire_ms` 50, `damage` 7, `range` 768, `knockback` 1.0 |
| `[rules]` | `mode` "vamp" / "classic", `lifesteal_percent` 100, `frag_limit` 10 (classic), `spawn_health` 100, `max_health` 100, `spawn_armor` 0, `respawn_delay_ms` 1000, `reset_winner_health` true, `hit_mode` "anti_lag" / "no_anti_lag", `max_rewind_ms` 200 |

Defaults reproduce the classic arena-shooter movement and the vamp LG duel (100% lifesteal, the winner of a
fight is reset to full health). Anti-lag judges hits where the shooter saw the target, up to `max_rewind_ms`
back (fully compensates pings up to ~140 ms with the default).

## Client (`settings.toml`)

Saved at `%APPDATA%\LGDuel\settings.toml` (Linux `~/.config/lgduel/`, macOS
`~/Library/Application Support/LGDuel/`). The Settings menu edits it; every key is also a console variable
(`` ` `` → `mouse.cm_per_360 30`, `cvars` lists them), and `writeconfig NAME` / `exec NAME` save and load
profiles. `lgduel --print-settings` prints the file with current values; command-line options override it
for one run.

| Section | Keys (default) |
|---|---|
| top level | `name` "player", `rendezvous` "" (join-code service; empty = shipped one) |
| `[mouse]` | `cm_per_360` 30, `dpi` 800, `invert_pitch` false |
| `[video]` | `fov` 110 (10–130, 4:3 horizontal), `fullscreen` false, `size` [1280, 720], `vsync` false, `fps_cap` (unset = uncapped; e.g. `fps_cap = 240`), `ui_scale` 1.0 (menu size, 0.75–2.0), `msaa` 1 (anti-aliasing: 1 = off, 2 or 4; applied at start-up, `--msaa 4`), `theme` "classic" (or "grid", or a folder in `themes/`, see [THEMES.md](THEMES.md)) |
| `[effects]` | `gun` "none" / "small" / "normal", `beam_style` "none" / "thin" / "thick" / "zigzag", `beam_color`, `sparks` true, `spark_size` 3, `spark_color` |
| `[hud]` | `damage_numbers` true, `damage_color`, `text_color`, `text_scale` 3, `text_shadow` true, `net_graph` true (F3), `net_graph_color`, `input_display` false, `auto_scale` false (HUD text scales with window height) |
| `[hud.crosshair]` | `shape` "cross" / "dot" / "circle" / "tshape", `size` 8, `gap` 4, `thickness` 2, `dot` true, `color`, `outline` false, `outline_color`, `hit_color` (shown for 0.15 s after your beam hits; same as `color` = no change) |
| `[enemy_colors]` | `head` yellow, `upper` green, `lower` cyan (solid, unshaded; used while `models.forced_colors` is on) |
| `[ui]` | `accent` colour of the menus (default blue) |
| `[models]` | `enemy` "classic" / "humanoid", `forced_colors` true (off = show the skin), `skin` (`head`, `torso`, `arms`, `legs` colours) |
| `[audio]` | `volume` 0.7, `hit_volume` 1.0, `hit_pitch_by_damage` false, `kill_sound` false, `ui_sounds` false, `hit_sound` / `kill_sound_file` (unset = synthesised; a file name in `sounds/` next to the settings, see [KOVAAKS.md](KOVAAKS.md); console `hitsound`; never part of a share code) |
| `[kovaaks]` | `path` "" (your KovaaK's folder; empty = find it through Steam, see [KOVAAKS.md](KOVAAKS.md)) |
| `[practice]` | `bot` true, `pattern` "still"/"ad"/"mirror"/"anti"/"hsw"/"hswanti", `skill` "easy"/"medium"/"hard"/"aimbot", `map` |
| `[host]` | Host-a-game screen: `name`, `port`, `classic`, `lifesteal_percent`, `anti_lag`, `map`, `join_code` |
| `[filter]` | server browser: `mode`, `anti_lag`, `max_ping` (0 = any), `region`, `hide_empty`, `hide_full` |
| `[[servers]]` | servers you added in the browser (`name`, `address`) |

Colours are `[r, g, b]` from 0 to 1. Out-of-range values are clamped when loading (e.g. `fov = 500` → 130).

## Shipped server list (`servers.toml`)

Ships with the game: `[[server]]` entries (`name`, `address`) shown in everyone's browser, and `rendezvous`,
the default join-code / public-list service.

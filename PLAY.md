# Playing LGDuel

LGDuel is a Lightning-Gun duel trainer: two players, LG only, vamp mode by default (damage you deal heals you).

Unzip `LGDuel-windows-x64.zip` and run **`lgduel.exe`**. Windows 10/11, any GPU that supports Vulkan or
DirectX 12 (most from the last ~8 years). The main menu has everything below.

## Main menu

| | |
|---|---|
| **Practice** | offline duel against a bot: pick the map, how it strafes (still, A/D, mirror, anti-mirror, half-sideways…) and how well it aims |
| **Play online** | server list with ping, players, map and rules. **Join** a server, add your own (name + `address:port`), or direct-connect to an address |
| **Host a game** | runs a server on this PC and joins it; friends connect to the address it shows |
| **Settings** | see below (also reachable from the Esc menu in game) |

## Playing online

**On a dedicated server** (one listed under Play online, or any `address:port` you were given) there is
nothing to set up: click **Join**. A server runs several duels (arenas) at once:

- you are put in an arena with someone who is waiting, or in a free arena;
- alone, you get a **practice bot** after 2 seconds; it leaves when a real opponent joins;
- when every arena is full you **queue** and watch a duel until a place opens;
- **Esc → Spectate** watches instead of playing; **Esc → Play** gets back in the queue.

The line at the top of the screen says where you are: `ARENA 2 - VS NAME`, `QUEUE #1 - WATCHING ARENA 1`, …

**Hosting yourself** (Host a game): friends on the same network see your game under Play online.
Over the internet the easiest way is a **join code**: with *Join code* ticked on the Host screen, your
Esc menu shows a code like `LG-7KQ2`; friends type it under Play online → *Join code* and connect without
any port forwarding (the game opens a direct path through both routers, or uses the relay when the routers
don't allow that). Join codes need a rendezvous service: the one shipped with the game, or one you run
yourself (`lgduel-server relay`, see [DEPLOY.md](DEPLOY.md)) and type under *via*.

Without join codes, friends need your public IP with **UDP port 27960 forwarded** on your router, or a VPN
like Tailscale / ZeroTier. Windows may ask to allow `lgduel.exe` through the firewall: allow it. A dedicated
server on a small VPS avoids all of this, see [DEPLOY.md](DEPLOY.md).

## Controls

| | |
|---|---|
| Mouse | look (raw input) |
| W A S D | move |
| Space | jump (hold to keep hopping) |
| Ctrl or C | crouch |
| Left mouse | Lightning Gun |
| Esc | menu (releases the mouse) |
| F3 | net-graph on/off |
| F11 or Alt+Enter | fullscreen on/off |
| ` (key left of 1) | console |

## Settings

Changes apply immediately and are saved to `%APPDATA%\LGDuel\settings.toml`.

- **Mouse:** sensitivity as cm per 360°, mouse DPI, invert Y.
- **Video:** field of view, fullscreen, window size, vsync, FPS cap.
- **Visuals:** beam style (none / thin / thick / zigzag), impact sparks and their size, enemy colours
  (head / body / legs; default yellow / green / cyan so they stand out), crosshair editor, net-graph.
- **Audio:** hit-sound volume (with a test button).
- **Player:** your name.

Command-line options still work and override the file for that run, e.g.
`lgduel.exe --sens-cm 55 --dpi 800`, `--fov 110`, `--map arena_tiers --practice`, `--connect 1.2.3.4:27960`.

## Console and profiles

**`** opens the console. Every setting is a console variable named like in `settings.toml`:
`mouse.cm_per_360 30`, `video.fov 110`, `hud.crosshair.size 10`, `effects.beam_style zigzag`
(short names: `sensitivity`, `dpi`, `fov`, `volume`, `beam`, `name`). `cvars` lists them all,
`cvars crosshair` filters. Changes apply immediately and are saved.

`writeconfig NAME` saves all your settings as `NAME.cfg` next to `settings.toml`; `exec NAME` loads it
again. Profiles are plain text (`set KEY VALUE` lines), so you can keep several setups and share them.
The console also does `connect ADDRESS`, `join CODE`, `practice`, `disconnect` and `quit`.

## Server rules (hosting)

The Host screen sets name, map, port, mode (vamp / classic), lifesteal % and anti-lag. Everything else
(tick rate, physics, LG damage/range, arenas, practice bot) is in a config file: copy
`server.example.toml`, edit it, and run `lgduel.exe host --config server.toml` (or `lgduel-server.exe`
with the same options). Anti-lag (default on) makes hits register where *you* saw the opponent;
`hit_mode = "no_anti_lag"` uses the server's current positions instead.

## What to look at / report

The **net-graph** (bottom-right) shows ping, jitter, packet loss, server tick rate, snapshots per second,
interpolation delay, bandwidth, `HITREG` (how often the server agreed with what your screen showed when
you fired) and `RB` (rubber-band corrections of your own view). Useful feedback:

- ping / loss / HITREG / RB values during play, and whether movement and hits *felt* right;
- whether the hit sound is audible and not annoying;
- anything that crashed or looked wrong (a screenshot helps).

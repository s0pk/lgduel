## LGDuel v0.2.0

**Downloads:** `LGDuel-windows-x64.zip` (the game, unzip and run `lgduel.exe`) ·
`lgduel-server-linux-x64.tar.gz` (dedicated server for a Linux VPS, see [DEPLOY.md](DEPLOY.md)).

### New

- **Menus.** `lgduel.exe` opens a main menu: Practice, Play online, Host a game, Settings, Quit.
  **Esc** in game opens the pause menu (Resume / Settings / Spectate / Disconnect / Quit).
- **Settings in game**, saved to `%APPDATA%\LGDuel\settings.toml`: mouse (cm/360, DPI, invert),
  video (FOV, fullscreen, window size, vsync, FPS cap), visuals (beam style, sparks, enemy colours,
  crosshair editor, net-graph), audio, player name. **F11** or **Alt+Enter** toggles borderless fullscreen.
- **Server browser.** Play online lists servers with ping, map, players and rules; add your own
  servers or direct-connect to an address.
- **Host a game** from the menu: runs a server inside the game and joins it.
- **Maps with height:** `arena_pillars` (pillars, raised centre with ramps and stairs) and `arena_tiers`
  (three levels joined by ramps and stairs), next to `arena_flat`. Ramps use proper sloped collision.
- **Dedicated servers for many players:** a server runs up to 8 separate duels (arenas) at once with
  an automatic queue. Players left alone get paired up; a lone player gets a practice bot until a
  real opponent joins. Spectate from the Esc menu. The top of the screen shows your arena, opponent,
  queue position or what you are watching.
- **VPS deploy kit:** one-command install as a systemd service, or Docker (`DEPLOY.md`).
- Hit sounds, damage numbers and kill notices are now delivered reliably, even with packet loss.

### Compatibility

Protocol version 5: v0.2 clients and servers don't talk to v0.1. Settings from command-line options
still work and override the settings file.

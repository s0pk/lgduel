## LGDuel v0.2.3

**Download `LGDuel-windows-x64.zip`, unzip, run `lgduel.exe`.** Guide in `PLAY.md`. Same network protocol as v0.2.0-v0.2.2, so everyone can still play each other and the Sydney server.

**New: the Quake look.** Settings -> Share -> Presets -> **quake** switches all of this on at once; each part also has its own setting.
- **Quake theme.** Dark stone blocks, riveted steel floor plates, tread-plate ramps and ribbed ceilings, all drawn by the game itself (no assets from any game). Floors stay sharp at shallow angles. Enemies in their bright colours still stand out.
- **Beam from the bottom middle.** Settings -> Visuals -> Beam -> *Starts at*: *Bottom middle* starts your own beam on the bottom edge of the screen, straight under the crosshair, at any window size or FOV.
- **Fighter enemy model.** Boots, knees, shoulder pads, a visor and a Lightning Gun held in both hands. Settings -> Visuals -> Enemies -> Model.
- **Legs that follow how the player really moves.** On the fighter the legs turn toward the direction of travel (strafing, diagonals), run backwards when backing up, stride in step with the distance covered, tuck in the air, and keep the lowest foot on the floor.

**Notes**
- Defaults are unchanged: the classic look and classic model stay unless you pick something else.
- Built on Linux (cross-compiled), checked under Wine, not yet on a real Windows PC. The animation was verified from rendered frames and tests, not yet watched live with two players.

## LGDuel v0.2.2

**Download `LGDuel-windows-x64.zip`, unzip, run `lgduel.exe`.** Guide in `PLAY.md`. Same network protocol as v0.2.0 and v0.2.1.

**New**
- **An official Sydney server.** *Play online* now lists **Sydney Duels (OC)**: click Join, no setup, no port forwarding. Up to 8 duels at once with a queue; a lone player gets a practice bot until an opponent joins.
- **Join codes work out of the box.** The game ships with the same Sydney machine as its join-code relay, so *Host a game* gives friends a code (`LG-7KQ2`) even when your router can't open ports. The game tries a direct connection first and only relays if it has to.
- **Run your own server in one command** on a Linux VPS (Debian, Ubuntu, AlmaLinux): `git clone ... && cd lgduel && sudo ./deploy/install.sh` builds it, starts the game server and the relay, and prints the address (`DEPLOY.md`).

**Notes**
- Built on Linux (cross-compiled), checked under Wine, not yet on a real Windows PC.
- The shipped server is a single machine in Sydney; players far away will see higher ping.

## LGDuel v0.2.1

**Download `LGDuel-windows-x64.zip`, unzip, run `lgduel.exe`.** Guide in `PLAY.md`. Same network protocol as v0.2.0, so both versions can play each other.

**New**
- **Host without port forwarding.** *Host a game* now asks your router to open the port itself (UPnP / NAT-PMP) and shows a **direct code** like `LG-4F7K-Q29M-XTR` in the Esc menu. Friends paste it under *Play online → Join code*. No account, no extra service. It needs UPnP switched on in your router and a router with a public address; when it can't, the game says why and you can still forward the port by hand, use a VPN, or use a join code.
- **One-command server:** `host.cmd` (Windows) / `host.sh` start a dedicated server, print the addresses to share and open the port the same way.
- `lgduel-server --upnp`, `server.upnp = true`, `lgduel --join CODE` accepts direct codes.

**Notes**
- Built on Linux (cross-compiled), checked under Wine, not yet on a real Windows PC. The router port opening was tested against simulated routers only; please report what your router does.
- Direct codes contain the host's public IP: share them with friends, not publicly.

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

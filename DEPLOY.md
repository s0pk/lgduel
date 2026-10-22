# Running an LGDuel server on a VPS

A dedicated server lets anyone join from the in-game server list: no port forwarding or VPN on the
players' side. One small Linux VPS runs up to 8 separate duels at once (16 players) plus a queue.

## What you need

- A Linux VPS with a public IPv4 address, x86-64, Debian 12 or Ubuntu 22.04/24.04.
  The smallest plan is plenty (1 vCPU, 512 MB–1 GB RAM). Pick a location close to your players:
  ping matters more than anything else in an LG duel.
- Traffic: about 16 KB/s per connected player each way, so a full server is about 2 Mbit/s.

## 1. Open the game port in the provider's firewall

Most providers have a firewall / security group in their web panel. Allow **inbound UDP 27960**
(from anywhere). TCP is not used.

## 2. Install (one command)

SSH into the VPS and run:

```bash
curl -fsSL https://raw.githubusercontent.com/s0pk/lgduel/main/deploy/install.sh | sudo bash
```

This downloads the latest release's `lgduel-server-linux-x64.tar.gz`, installs it to `/opt/lgduel`,
creates a `lgduel` system user, writes `/etc/lgduel/server.toml` (only if it doesn't exist yet) and
starts the `lgduel-server` systemd service. The service restarts on crashes and on reboot.
If `ufw` is active, it also allows UDP 27960 there.

## 3. Check it works

```bash
journalctl -u lgduel-server -f
```

shows `lgduel-server 'LGDuel server' listening on 0.0.0.0:27960 …` and later one line per
connect / disconnect / arena change.

On your PC: start LGDuel → **Play online** → *Add a server to the list*: name + `YOUR.VPS.IP:27960` →
the row should show a ping and `0/16` players. Click **Join**. Alone you get the
practice bot after 2 seconds; the next player to join takes its place.

To put it in everyone's list, add it to `config/servers.toml` in this repository (that file ships with
the game).

## Settings

Edit `/etc/lgduel/server.toml`, then `sudo systemctl restart lgduel-server`. Every key is optional;
`config/server.example.toml` lists all of them with their defaults. The useful ones:

```toml
[server]
name = "My LGDuel server"   # shown in the server browser
max_arenas = 8              # separate duels; 2 players each, the rest queue
map = "arena_flat"          # arena_flat | arena_pillars | arena_tiers
practice_bot = true         # a lone player gets a bot until an opponent joins

[rules]
mode = "vamp"               # "vamp" | "classic"
lifesteal_percent = 100
hit_mode = "anti_lag"       # "anti_lag" | "no_anti_lag"
```

If you change `port`, open that UDP port in the provider firewall instead of 27960.

## Admin commands (no restart needed)

Run commands remotely with a password: set `rcon_password = "something long"` under `[server]` in
`/etc/lgduel/server.toml`, restart once, then from any PC with LGDuel:

```bash
lgduel-server rcon --password "something long" YOUR.VPS.IP status
```

(or put the password in the `LGDUEL_RCON_PASSWORD` environment variable). When you run the server in
a terminal yourself, type the same commands there.

| Command | |
|---|---|
| `status` | players, their arena, ping and loss |
| `kick <name or #id> [reason]` | disconnect a player (they see the reason) |
| `say <text>` | message on everyone's screen |
| `get rules.lifesteal_percent` | show a setting |
| `set rules.lifesteal_percent 50` | change a setting for everyone right away (validated; not saved to the file) |
| `reload` | re-read `server.toml` |

Rules, LG and server-browser settings change live. Physics, tick rate, map, port and `max_arenas` need a
restart (connected players' movement prediction depends on them). Wrong passwords get no answer, and
an address that guesses wrong 5 times is ignored for a minute.

## Update

Run the install command again: it replaces the binary, keeps your `server.toml` and restarts the service.

## Docker instead of systemd

```bash
git clone https://github.com/s0pk/lgduel && cd lgduel
cp config/server.example.toml server.toml     # edit it
docker compose up -d
```

The image builds the server from source (a few minutes the first time) and publishes UDP 27960.

## Commands

| What | Command |
|---|---|
| Logs | `journalctl -u lgduel-server -f` |
| Restart | `sudo systemctl restart lgduel-server` |
| Stop / start | `sudo systemctl stop lgduel-server` / `start` |
| Remove | `sudo systemctl disable --now lgduel-server && sudo rm -rf /opt/lgduel /etc/lgduel /etc/systemd/system/lgduel-server.service` |

## Security notes

- The server runs as an unprivileged `lgduel` user with a read-only view of the system (systemd hardening).
- It only listens on one UDP port. Connection attempts need a cookie round trip (no spoofed slots), and
  browser queries are padded so the server never answers with more bytes than it received.

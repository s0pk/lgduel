# Hosting LGDuel games

Pick the way that fits; all of them run the same server code.

| You want… | Do this | Port forwarding? |
|---|---|---|
| A quick duel with a friend | **Host a game** in the main menu, keep *Join code* ticked, send the code from your Esc menu | No (join code) |
| Friends on the same network | Host a game; they see it under Play online (LAN) | No |
| A server that's always up for everyone | A dedicated server on a small Linux VPS: [DEPLOY.md](DEPLOY.md) | No (it has a public IP) |
| A dedicated server on your own PC | `lgduel-server.exe --config server.toml` (or `lgduel.exe host …`) | Yes: UDP 27960, or use a join code |

## Host a game (in the client)

Main menu → **Host a game**: server name, map, port, mode (vamp / classic), lifesteal %, anti-lag, join
code. *Start hosting* runs the server inside the game and joins it. The Esc menu shows how others get in:
your LAN address and, with a rendezvous service, a join code (`LG-7KQ2`). Leaving the game stops the server.

## Direct codes (the game opens the port itself)

With *Open port* ticked (default) on the Host screen, or `--upnp` / `upnp = true` for `lgduel-server`
(`host.sh` / `host.cmd` pass it), the host asks the router to forward its UDP port: UPnP IGD first, then
NAT-PMP. If the router agrees and its own address is public, the host prints `DIRECT CODE LG-4F7K-Q29M-XTR`
and the Esc menu shows it. The code is the public address and port plus check bits, so friends join with
**Play online → Join code** (or `lgduel --join CODE`) without any rendezvous service. The mapping is renewed
every 30 minutes and removed when hosting stops.

It fails, with the reason printed, when UPnP/NAT-PMP is off in the router, or when the router's own address is
not public (carrier-grade NAT, double NAT): then use a join code below, a VPN, or forward the port by hand.
The code contains the host's public IP: share it with the people you play with, not publicly.

## Join codes (no port forwarding)

Join codes need a **rendezvous service** (`lgduel-server relay`, UDP 27950) that both players can reach. It
introduces the two PCs so they open a direct path through their routers ("hole punching"); when the routers
don't allow that (symmetric NATs, strict firewalls), it relays the game traffic. Players use the service
shipped in the game's `servers.toml`, or type one under Play online → Join code → *via*. Run your own on a
VPS with [DEPLOY.md](DEPLOY.md) (the installer sets it up next to the game server).

## Dedicated server (`lgduel-server`)

```
lgduel-server [--config server.toml] [--port N] [--bind ADDR] [--map NAME|FILE]
              [--rendezvous HOST[:PORT]] [--upnp] [--duration-secs N] [--stats-file out.toml]
lgduel-server relay [--port 27950] [--bind ADDR]
lgduel-server rcon [--password PW] ADDRESS COMMAND...
```

`lgduel.exe host …` takes the same options. Copy `server.example.toml`, edit it, pass it with `--config`;
every key is described in [CONFIG.md](CONFIG.md). One server runs `max_arenas` separate duels (default 8):
players are paired automatically, extras queue and watch, a lone player gets a practice bot, and players can
challenge each other from the Esc menu.

Typed into the server's terminal (or remotely with `lgduel-server rcon`, after setting `rcon_password`):
`status`, `kick NAME [reason]`, `say TEXT`, `get section.key`, `set section.key VALUE`, `reload`, `help`.
Rules and LG settings change live; physics, tick rate, map, port and arena count need a restart.

`--stats-file out.toml` writes every player's stats (accuracy, damage, frags, fights) when the server stops.

## Public server list

A dedicated server with `rendezvous` set and `public = true` appears in the server browser of everyone using
that rendezvous service, with its `region` shown and filterable. Without it, players add the server by
address (Play online → *Add a server*), and servers on the same network show up by themselves.

## Ports

| | Protocol | Port |
|---|---|---|
| Game server | UDP | 27960 (`port`) |
| Rendezvous / relay / public list | UDP | 27950 |
| LAN discovery (browser asks) | UDP | 27960–27963 |

Nothing uses TCP.

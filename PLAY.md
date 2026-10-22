# Playing LGDuel with a friend

LGDuel is a Lightning-Gun duel trainer: two players, LG only, vamp mode (damage you deal heals you).
This is an early test build: no menus yet, everything is set with command-line options.

Everything is in **one file, `lgduel.exe`**. Both players need it. Windows 10/11, any GPU that supports
Vulkan, DirectX 12 or Metal (most from the last ~8 years).

## 1. One player hosts

Open a terminal (PowerShell or cmd) in the folder with `lgduel.exe` and run:

```
lgduel.exe host
```

It prints the exact command your friend should use, for example:

```
  players on this PC:      lgduel --connect 127.0.0.1:27960
  players on your network: lgduel --connect 192.168.1.45:27960
```

Windows may ask whether to allow `lgduel.exe` through the firewall: allow it (private networks are
enough for LAN/VPN play; allow public too if you play over the internet). Leave this window open while you play.

## 2. Both players connect

The host also plays, from a second terminal:

```
lgduel.exe --connect 127.0.0.1:27960 --name host
```

The friend uses one of these, depending on how you are connected:

| You are… | Friend runs | What the host needs to do |
|---|---|---|
| on the same network (LAN/Wi-Fi) | `lgduel.exe --connect <host's LAN IP>:27960` | nothing |
| far apart, using **Tailscale** / ZeroTier (easiest) | `lgduel.exe --connect <host's Tailscale IP>:27960` | both install Tailscale and log in to the same tailnet |
| far apart, plain internet | `lgduel.exe --connect <host's public IP>:27960` | forward **UDP** port 27960 on the router to the host PC |

Join codes that work without any of this are planned (they need a small rendezvous/relay service).

## 3. Controls

| | |
|---|---|
| Mouse | look (raw input) — click the window to capture the mouse, **Esc** releases it |
| W A S D | move |
| Space | jump (hold to keep hopping) |
| Ctrl or C | crouch |
| Left mouse | Lightning Gun |
| F3 | net-graph on/off |

## 4. Settings (command-line options)

Add these to the `--connect` command, e.g. `lgduel.exe --connect 1.2.3.4:27960 --name me --sens-cm 30 --dpi 800 --fov 110`.

| Option | Meaning | Default |
|---|---|---|
| `--name NAME` | your name | player |
| `--sens-cm N` | mouse sensitivity in cm per 360° | 30 |
| `--dpi N` | your mouse DPI | 800 |
| `--invert` | invert mouse Y | off |
| `--fov N` | field of view (10–130, Quake-style 4:3 horizontal) | 110 |
| `--size WxH` | window size | 1280x720 |
| `--beam none\|thin\|thick\|zigzag` | LG beam style | thin |
| `--no-sparks`, `--spark-size N` | impact sparks | on, 3 |
| `--volume 0..1`, `--no-sound` | hit sound volume | 0.7 |
| `--vsync`, `--fps-cap N` | frame pacing | uncapped, vsync off |
| `--no-net-graph` | hide the network overlay | shown |

Opponents are drawn in solid colours (yellow head, green body, cyan legs) so they stand out.

## 5. Practice alone

`lgduel.exe` without `--connect` starts an offline duel against a bot:
`--bot still|ad|mirror|anti|hsw|hswanti` picks how it strafes, `--bot-skill easy|medium|hard|aimbot` how well it aims.

## 6. Server options (host)

`lgduel.exe host --port N` changes the port. For other rules (non-vamp duels, lifesteal %, anti-lag on/off,
tick rate…) copy `server.example.toml` to `server.toml`, edit it, and run `lgduel.exe host --config server.toml`.
Anti-lag (default on) makes hits register where *you* saw the opponent; `hit_mode = "no_anti_lag"` uses the
server's current positions instead.

## 7. What to look at / report

The **net-graph** (bottom-right) shows ping, jitter, packet loss, server tick rate, snapshots per second,
interpolation delay, bandwidth, `HITREG` (how often the server agreed with what your screen showed when you
fired) and `RB` (rubber-band corrections of your own view). Useful feedback:

- ping / loss / HITREG / RB values during play, and whether movement and hits *felt* right;
- whether the hit sound is audible and not annoying;
- anything that crashed or looked wrong (a screenshot helps).

When you close the game window it prints stats to the terminal.

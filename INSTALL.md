# Installing LGDuel

## Arch Linux (AUR)

```bash
yay -S lgduel            # or: paru -S lgduel
```

Without an AUR helper:

```bash
git clone https://aur.archlinux.org/lgduel.git && cd lgduel
makepkg -si
```

This builds the game from source (a few minutes) and installs `lgduel` (the game and a practice bot, `lgduel-bot`) and
`lgduel-server` (dedicated server, see below). Then run **`lgduel`** or start LGDuel from your application menu.

**Install a graphics driver and sound support if you haven't:**

| You have | Install |
|---|---|
| AMD GPU | `vulkan-radeon` |
| NVIDIA GPU | `nvidia-utils` |
| Intel GPU | `vulkan-intel` |
| PipeWire (default on current Arch) | `pipewire-alsa` |
| PulseAudio | `pulseaudio-alsa` |

LGDuel needs Vulkan (or, as a fallback, OpenGL). Without a sound device it still runs, muted.

### Wayland and X11
Both work. The mouse is captured with the compositor's pointer-lock (Wayland: needs `pointer-constraints`, which GNOME, KDE,
Sway and Hyprland have) or confined to the window (X11). Raw mouse motion is used on both, so there is no acceleration.

### Where files live
| | |
|---|---|
| Settings | `~/.config/lgduel/settings.toml` |
| Your themes | `~/.config/lgduel/themes/` or `~/.local/share/lgduel/themes/` |
| Shipped example theme | `/usr/share/lgduel/themes/example` |
| Demos | `~/.config/lgduel/demos/` |

### Running a server
```bash
sudo systemctl enable --now lgduel-server     # game server on UDP 27960, config in /etc/lgduel/server.toml
sudo systemctl enable --now lgduel-relay      # join-code relay on UDP 27950 (optional)
```
Open those UDP ports in your firewall. See [DEPLOY.md](DEPLOY.md) and [HOSTING.md](HOSTING.md).

### Troubleshooting
- `lgduel --doctor` prints what the game sees (session type, GPU adapter, sound device, folders). Include it in bug reports.
- No window / "no GPU adapter": check `vulkaninfo --summary` (package `vulkan-tools`); try `WGPU_BACKEND=gl lgduel`.
- No sound: install `pipewire-alsa` (or `pulseaudio-alsa`); the game runs muted without it.
- Mouse not captured on Wayland: the compositor must support pointer-constraints.

### Uninstall
`sudo pacman -R lgduel lgduel-server`. Your settings in `~/.config/lgduel` stay until you delete them.

## Other Linux distributions
Build from source with Rust (stable, 1.99 or newer):

```bash
# Debian/Ubuntu: sudo apt install build-essential pkg-config libasound2-dev libvulkan1 mesa-vulkan-drivers
# Fedora:        sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel vulkan-loader mesa-vulkan-drivers
git clone https://github.com/s0pk/lgduel && cd lgduel
cargo build --profile dist -p lgduel-client -p lgduel-bot -p lgduel-server
target/dist/lgduel
```
Prebuilt Linux downloads, Flatpak and AppImage are not available yet.

## Windows
Download `LGDuel-windows-x64.zip` from the [Releases](https://github.com/s0pk/lgduel/releases), unzip it and run `lgduel.exe`. See [PLAY.md](PLAY.md).

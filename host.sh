#!/usr/bin/env bash
# One command to host an LGDuel server: ./host.sh [server options, e.g. --port 27960 --map arena_tiers]
# Builds the server if needed (no GPU or graphics libraries required), prints how players connect, runs it.
set -euo pipefail
cd "$(dirname "$0")"

PORT=27960
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
    [ "${args[i]}" = "--port" ] && PORT="${args[i + 1]:-$PORT}"
done

# 1. A server binary: the release zip's, else build one from source.
EXE=""
for c in ./lgduel-server ./lgduel-server.exe; do [ -x "$c" ] && EXE="$c" && break; done
if [ -z "$EXE" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
    if ! command -v cargo >/dev/null; then
        echo "Rust is needed to build the server (https://rustup.rs)."
        read -r -p "Install it now with rustup? [y/N] " yn || yn=n
        case "$yn" in
            y | Y) curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
                   export PATH="$HOME/.cargo/bin:$PATH" ;;
            *) echo "Install Rust, or download the release zip (it has lgduel-server) and run this script next to it."; exit 1 ;;
        esac
    fi
    echo "Building the server (first time takes a minute or two)..."
    cargo build --release -p lgduel-server
    EXE="${CARGO_TARGET_DIR:-target}/release/lgduel-server"
fi

# 2. Addresses (best effort).
LAN=""
if command -v hostname >/dev/null; then LAN=$(hostname -I 2>/dev/null | awk '{print $1}' || true); fi
[ -z "$LAN" ] && command -v ip >/dev/null && LAN=$(ip route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") {print $(i + 1); exit}}' || true)
[ -z "$LAN" ] && command -v ipconfig >/dev/null && LAN=$(ipconfig getifaddr en0 2>/dev/null || true)
PUB=""
if command -v curl >/dev/null; then PUB=$(curl -fsS --max-time 4 https://api.ipify.org 2>/dev/null || true)
elif command -v wget >/dev/null; then PUB=$(wget -qO- --timeout=4 https://api.ipify.org 2>/dev/null || true); fi

cat <<INFO

==================== LGDuel server ====================
 Players connect with the address below. In the game:
   Play online -> "Add a server" -> paste the address -> Join
 or from a terminal:  lgduel --connect ADDRESS

   This PC:        127.0.0.1:$PORT
   Same network:   ${LAN:-(not found)}:$PORT
   Internet:       ${PUB:-(not found, look up your public IP)}:$PORT

 At home the server asks your router to open the port by itself (UPnP / NAT-PMP). If that
 works it prints a "DIRECT CODE" below: friends paste it under Play online -> Join code, and
 the address above isn't needed. If your router says no, forward UDP $PORT there yourself.
 On a VPS: open UDP $PORT (nothing uses TCP) in the firewall (e.g. 'sudo ufw allow $PORT/udp')
 and in the provider's panel.
 Type 'help' here for server commands (status, kick, say, set ...). Ctrl+C stops the server.
=======================================================

INFO
exec "$EXE" --upnp "$@"

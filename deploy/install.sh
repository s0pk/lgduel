#!/usr/bin/env bash
# Install or update the LGDuel server on a Linux VPS (Debian/Ubuntu, x86-64) as systemd
# services: the game (UDP 27960) and the join-code relay + public server list (UDP 27950).
#
# From a clone of the repository (builds the server from source, ~2-5 minutes the first time):
#   git clone https://github.com/s0pk/lgduel.git && cd lgduel && sudo ./deploy/install.sh
# Or, once a release carries the Linux server tarball:
#   curl -fsSL https://raw.githubusercontent.com/s0pk/lgduel/main/deploy/install.sh | sudo bash
#
# Re-running updates the binary and keeps /etc/lgduel/server.toml.
# Environment:
#   LGDUEL_TARBALL=/path/to/lgduel-server-linux-x64.tar.gz   install this file instead of building
#   LGDUEL_REGION=OC      region label shown in the server browser (first install only)
#   LGDUEL_NAME="My server"   server name shown in the browser (first install only)
#   DESTDIR=/some/dir     stage the files there; no user, systemd or firewall changes (testing)
set -euo pipefail
REPO="${LGDUEL_REPO:-s0pk/lgduel}"
URL="https://github.com/$REPO/releases/latest/download/lgduel-server-linux-x64.tar.gz"
DESTDIR="${DESTDIR:-}"

[ -n "$DESTDIR" ] || [ "$(id -u)" = 0 ] || { echo "run as root: sudo ./deploy/install.sh"; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Are we running from a clone of the repository?
SRC=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "$(dirname "${BASH_SOURCE[0]}")/../crates/server/Cargo.toml" ]; then
    SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi

build_from_source() {
    # A C linker and curl are needed; fetch them on Debian/Ubuntu if missing.
    if { ! command -v cc >/dev/null || ! command -v curl >/dev/null; } && command -v apt-get >/dev/null; then
        echo "installing build tools (apt)"
        apt-get update -qq
        DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential curl ca-certificates git
    fi
    command -v cc >/dev/null || { echo "need a C compiler (e.g. apt install build-essential)"; exit 1; }
    for d in "$HOME/.cargo/bin" "${SUDO_USER:+$(getent passwd "$SUDO_USER" | cut -d: -f6)/.cargo/bin}"; do
        [ -n "$d" ] && [ -x "$d/cargo" ] && PATH="$d:$PATH"
    done
    if ! command -v cargo >/dev/null; then
        echo "installing Rust (rustup, minimal profile)"
        curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
        PATH="$HOME/.cargo/bin:$PATH"
    fi
    # Build outside the clone so it stays clean; kept between runs for fast updates.
    export CARGO_TARGET_DIR="${LGDUEL_BUILD_DIR:-/var/tmp/lgduel-build}"
    # Small VPS (< 1 GB RAM): one compiler at a time so the build isn't killed.
    mem_kb=$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null || echo 4000000)
    [ "$mem_kb" -lt 1000000 ] && export CARGO_BUILD_JOBS=1
    echo "building lgduel-server (a few minutes the first time)"
    (cd "$SRC" && cargo build --release --locked -p lgduel-server)
    mkdir -p "$tmp/lgduel-server-linux-x64"
    cp "$CARGO_TARGET_DIR/release/lgduel-server" "$SRC/config/server.example.toml" \
        "$SRC/deploy/lgduel-server.service" "$SRC/deploy/lgduel-relay.service" \
        "$tmp/lgduel-server-linux-x64/"
}

if [ -n "${LGDUEL_TARBALL:-}" ]; then
    cp "$LGDUEL_TARBALL" "$tmp/server.tgz"
    tar -xzf "$tmp/server.tgz" -C "$tmp"
elif [ -n "$SRC" ]; then
    build_from_source
else
    echo "downloading $URL"
    if ! curl -fsSL "$URL" -o "$tmp/server.tgz"; then
        echo "no server download is published for this release. Clone the repository and build it instead:"
        echo "  git clone https://github.com/$REPO.git && cd lgduel && sudo ./deploy/install.sh"
        exit 1
    fi
    tar -xzf "$tmp/server.tgz" -C "$tmp"
fi
src="$tmp/lgduel-server-linux-x64"

[ -n "$DESTDIR" ] || id lgduel >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin lgduel
install -d "$DESTDIR/opt/lgduel" "$DESTDIR/etc/lgduel" "$DESTDIR/etc/systemd/system"
install -m 755 "$src/lgduel-server" "$DESTDIR/opt/lgduel/lgduel-server"
if [ ! -f "$DESTDIR/etc/lgduel/server.toml" ]; then
    install -m 644 "$src/server.example.toml" "$DESTDIR/etc/lgduel/server.toml"
    # First install: list this server publicly through the relay that runs on this same machine.
    sed -i -e 's|^rendezvous = ""|rendezvous = "127.0.0.1"|' -e 's|^public = false|public = true|' \
        "$DESTDIR/etc/lgduel/server.toml"
    [ -z "${LGDUEL_REGION:-}" ] || sed -i "s|^region = \"\"|region = \"${LGDUEL_REGION//[^A-Za-z0-9 -]/}\"|" "$DESTDIR/etc/lgduel/server.toml"
    [ -z "${LGDUEL_NAME:-}" ] || sed -i "s|^name = .*|name = \"${LGDUEL_NAME//[^A-Za-z0-9 ._-]/}\"|" "$DESTDIR/etc/lgduel/server.toml"
fi
install -m 644 "$src/lgduel-server.service" "$DESTDIR/etc/systemd/system/lgduel-server.service"
install -m 644 "$src/lgduel-relay.service" "$DESTDIR/etc/systemd/system/lgduel-relay.service"

if [ -n "$DESTDIR" ]; then
    echo "staged in $DESTDIR (no services started)"
    exit 0
fi
systemctl daemon-reload
systemctl enable --now lgduel-server lgduel-relay
systemctl restart lgduel-server lgduel-relay

# Open the game port if ufw is active (the provider's firewall is separate: see DEPLOY.md).
if command -v ufw >/dev/null && ufw status | grep -q "Status: active"; then
    ufw allow 27960/udp
    ufw allow 27950/udp
fi
sleep 1
systemctl --no-pager --lines=5 status lgduel-server lgduel-relay || true
ip=$(curl -fsS --max-time 5 https://api.ipify.org 2>/dev/null || true)
cat <<EOF

LGDuel server installed.
  Game:        UDP 27960   (players: Play online -> Add a server -> ${ip:-YOUR.VPS.IP}:27960)
  Join codes:  UDP 27950   (players: Play online -> Join code -> via ${ip:-YOUR.VPS.IP})
  IMPORTANT: allow inbound UDP 27960 and 27950 in your provider's firewall panel (TCP is not used).
  Config: /etc/lgduel/server.toml   Logs: journalctl -u lgduel-server -u lgduel-relay -f
  Update: git pull && sudo ./deploy/install.sh
EOF

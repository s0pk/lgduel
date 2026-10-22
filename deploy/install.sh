#!/usr/bin/env bash
# Install or update the LGDuel server on a Linux VPS (Debian/Ubuntu, x86-64) as a
# systemd service. Run as root:
#   curl -fsSL https://raw.githubusercontent.com/s0pk/lgduel/main/deploy/install.sh | sudo bash
# Re-running updates the binary and keeps /etc/lgduel/server.toml.
# LGDUEL_TARBALL=/path/to/lgduel-server-linux-x64.tar.gz installs a local file instead.
set -euo pipefail
REPO="${LGDUEL_REPO:-s0pk/lgduel}"
URL="https://github.com/$REPO/releases/latest/download/lgduel-server-linux-x64.tar.gz"

[ "$(id -u)" = 0 ] || { echo "run as root (sudo)"; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ -n "${LGDUEL_TARBALL:-}" ]; then
    cp "$LGDUEL_TARBALL" "$tmp/server.tgz"
else
    echo "downloading $URL"
    curl -fsSL "$URL" -o "$tmp/server.tgz"
fi
tar -xzf "$tmp/server.tgz" -C "$tmp"
src="$tmp/lgduel-server-linux-x64"

id lgduel >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin lgduel
install -d /opt/lgduel /etc/lgduel
install -m 755 "$src/lgduel-server" /opt/lgduel/lgduel-server
[ -f /etc/lgduel/server.toml ] || install -m 644 "$src/server.example.toml" /etc/lgduel/server.toml
install -m 644 "$src/lgduel-server.service" /etc/systemd/system/lgduel-server.service
systemctl daemon-reload
systemctl enable --now lgduel-server
systemctl restart lgduel-server

# Open the game port if ufw is active (the provider's firewall is separate: see DEPLOY.md).
if command -v ufw >/dev/null && ufw status | grep -q "Status: active"; then
    ufw allow 27960/udp
fi
sleep 1
systemctl --no-pager --lines=5 status lgduel-server || true
echo "LGDuel server installed. Config: /etc/lgduel/server.toml  Logs: journalctl -u lgduel-server -f"

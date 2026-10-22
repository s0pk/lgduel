#!/usr/bin/env bash
# Build a release package for friend tests: dist/LGDuel-<os>-x64.zip
# containing lgduel(.exe), lgduel-server(.exe), lgduel-bot(.exe), PLAY.md and
# server.example.toml. Then smoke-test the packaged binaries.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || export PATH="$HOME/.cargo/bin:$PATH"

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) OS=windows; EXE=.exe ;;
    Darwin) OS=macos; EXE= ;;
    *) OS=linux; EXE= ;;
esac
NAME="LGDuel-$OS-x64"
OUT="dist/$NAME"
T="${CARGO_TARGET_DIR:-target}"

cargo build --profile dist -p lgduel-client -p lgduel-server -p lgduel-bot
rm -rf "$OUT" "dist/$NAME.zip"
mkdir -p "$OUT"
for b in lgduel lgduel-server lgduel-bot; do
    cp "$T/dist/$b$EXE" "$OUT/"
done
cp PLAY.md README.md HOSTING.md DEPLOY.md CONFIG.md MAPS.md LICENSE "$OUT/"
cp config/server.example.toml "$OUT/"
git rev-parse --short HEAD > "$OUT/VERSION.txt" 2>/dev/null || echo unknown > "$OUT/VERSION.txt"

# Smoke test the packaged binaries: host + bot + headless client.
P=27991
"$OUT/lgduel$EXE" host --bind 127.0.0.1 --port $P --duration-secs 7 > dist/smoke_host.log 2>&1 &
sleep 0.5
"$OUT/lgduel-bot$EXE" --server 127.0.0.1:$P --pattern ad --name smokebot --duration-secs 5 > dist/smoke_bot.log 2>&1 &
"$OUT/lgduel$EXE" --headless --connect 127.0.0.1:$P --name packaged --secs 4 --size 1280x720 | grep -E "ONLINE CHECK OK|NETGRAPH"
wait
grep -q "packaged connected" dist/smoke_host.log

python - "$OUT" "dist/$NAME.zip" <<'PY'
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
base = os.path.basename(src)
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for name in sorted(os.listdir(src)):
        z.write(os.path.join(src, name), f"{base}/{name}")
PY
ls -l "$OUT" "dist/$NAME.zip"
echo "PACKAGE OK: dist/$NAME.zip"

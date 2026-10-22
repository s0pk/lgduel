#!/usr/bin/env bash
# Full local check: fmt, clippy, build, tests, headless sims.
# Every stage must pass. Stages that do not exist yet print an explicit SKIP
# with the PLAN.md phase that adds them; never stub a pass.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || export PATH="$HOME/.cargo/bin:$PATH"

PROFILE=release-dbg
# Build output (a Linux container sharing this checkout uses its own).
T="${CARGO_TARGET_DIR:-target}"
start=$(date +%s)
stage() { printf '\n==> %s\n' "$*"; }

stage "cargo fmt --check"
cargo fmt --all -- --check

stage "cargo clippy -D warnings"
cargo clippy --workspace --all-targets --profile "$PROFILE" -- -D warnings

stage "cargo build ($PROFILE)"
cargo build --workspace --profile "$PROFILE"

stage "cargo test ($PROFILE)"
cargo test --workspace --profile "$PROFILE"

stage "determinism (recorded 60 s duel replayed, golden hash)"
cargo test -q -p lgduel-sim --profile "$PROFILE" --test determinism -- --nocapture 2>&1 | grep -E "DETERMINISM|test result"

stage "binaries run"
"$T/$PROFILE/lgduel" --version >/dev/null && echo "ok: lgduel --version"
"$T/$PROFILE/lgduel-server" --version >/dev/null && echo "ok: lgduel-server --version"
"$T/$PROFILE/lgduel-server" --bind 127.0.0.1 --port 0 --duration-secs 0.5 | tail -1
for bin in lgduel-bot lgduel-tools; do
    "$T/$PROFILE/$bin" >/dev/null
    echo "ok: $bin"
done

stage "headless match: server + 2 bots, 60 s vamp duel × {clean@arena_flat, lossy 2% 30±10 ms@arena_pillars, highping 100 ms@arena_tiers} in parallel"
"$T/$PROFILE/lgduel-tools" match --secs "${MATCH_SECS:-60}" --maps arena_flat,arena_pillars,arena_tiers --out artifacts

stage "online client smoke: real lgduel --headless --connect vs server + bot, and spectating 2 bots (parallel)"
"$T/$PROFILE/lgduel-tools" client-smoke --secs 5 --out artifacts

stage "netem matrix: {0,50,100,200} ms × {0,1,2.5,5}% loss × ±20 ms jitter (MemNet, thresholds asserted)"
cargo test -q -p lgduel-server --profile "$PROFILE" --test netem_matrix -- --nocapture 2>&1 | grep -E "^\||test result"

stage "p2p: join code via lgduel-server relay; bot direct, client forced through the relay (NAT cases: tests/p2p.rs)"
"$T/$PROFILE/lgduel-tools" p2p-smoke --secs 5 --out artifacts

stage "client offscreen render (1920x1080, screenshot + pixel checks)"
"$T/$PROFILE/lgduel" --headless --frames 300 --size 1920x1080 --screenshot artifacts/offscreen.png
# Visibility toggles: no-beam and sparks-off must draw zero beam / spark pixels; zigzag style renders.
"$T/$PROFILE/lgduel" --headless --frames 3 --beam none --no-sparks | grep -E "IMAGE (beam|sparks)|CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --beam zigzag --spark-size 6 | grep -E "IMAGE (beam|sparks)|CHECK"
# End-of-fight summary panel (sample numbers) over the duel scene.
"$T/$PROFILE/lgduel" --headless --frames 3 --summary-demo --size 1280x720 --screenshot artifacts/fight_summary.png | grep -E "CHECK"
# Every built-in map renders with the duel checks passing.
for m in arena_pillars arena_tiers; do
  "$T/$PROFILE/lgduel" --headless --frames 3 --map "$m" --size 1280x720 --screenshot "artifacts/map_$m.png" | grep -E "^MAP|CHECK"
done
# egui pause menu (pixel font) over the scene.
"$T/$PROFILE/lgduel" --headless --frames 3 --menu pause --size 1280x720 --screenshot artifacts/menu.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu pause-online --size 1280x720 --screenshot artifacts/menu_online.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu settings --size 1280x720 --screenshot artifacts/settings.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu main --size 1280x720 --screenshot artifacts/mainmenu.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu online --size 1280x720 --screenshot artifacts/online_menu.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu host --size 1280x720 --screenshot artifacts/host_menu.png | grep -E "IMAGE menu|MENU CHECK"

printf '\nCHECK GREEN in %ss\n' "$(( $(date +%s) - start ))"

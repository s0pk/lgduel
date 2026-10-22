#!/usr/bin/env bash
# Full local check: fmt, clippy, build, tests, headless sims.
# Every stage must pass. Stages that do not exist yet print an explicit SKIP
# with the PLAN.md phase that adds them; never stub a pass.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || export PATH="$HOME/.cargo/bin:$PATH"

PROFILE=release-dbg
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
"target/$PROFILE/lgduel" --version >/dev/null && echo "ok: lgduel --version"
"target/$PROFILE/lgduel-server" --version >/dev/null && echo "ok: lgduel-server --version"
"target/$PROFILE/lgduel-server" --bind 127.0.0.1 --port 0 --duration-secs 0.5 | tail -1
for bin in lgduel-bot lgduel-tools; do
    "target/$PROFILE/$bin" >/dev/null
    echo "ok: $bin"
done

stage "headless match: server + 2 bots, 60 s vamp duel × {clean, lossy 2% 30±10 ms, highping 100 ms} in parallel"
"target/$PROFILE/lgduel-tools" match --secs "${MATCH_SECS:-60}" --out artifacts

stage "online client smoke: real lgduel --headless --connect vs server + bot"
"target/$PROFILE/lgduel-tools" client-smoke --secs 5 --out artifacts

stage "netem matrix: {0,50,100,200} ms × {0,1,2.5,5}% loss × ±20 ms jitter (MemNet, thresholds asserted)"
cargo test -q -p lgduel-server --profile "$PROFILE" --test netem_matrix -- --nocapture 2>&1 | grep -E "^\||test result"

stage "p2p rendezvous"
echo "SKIP: not implemented yet (PLAN Phase 7)"

stage "client offscreen render (1920x1080, screenshot + pixel checks)"
"target/$PROFILE/lgduel" --headless --frames 300 --size 1920x1080 --screenshot artifacts/offscreen.png
# Visibility toggles: no-beam and sparks-off must draw zero beam / spark pixels; zigzag style renders.
"target/$PROFILE/lgduel" --headless --frames 3 --beam none --no-sparks | grep -E "IMAGE (beam|sparks)|CHECK"
"target/$PROFILE/lgduel" --headless --frames 3 --beam zigzag --spark-size 6 | grep -E "IMAGE (beam|sparks)|CHECK"
# egui pause menu (pixel font) over the scene.
"target/$PROFILE/lgduel" --headless --frames 3 --menu --size 1280x720 --screenshot artifacts/menu.png | grep -E "IMAGE menu|MENU CHECK"

printf '\nCHECK GREEN in %ss\n' "$(( $(date +%s) - start ))"

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
# Gun model + input display on (both optional; default off).
"$T/$PROFILE/lgduel" --headless --frames 3 --gun normal --input-display --size 1280x720 --screenshot artifacts/gun_input.png | grep -E "CHECK"
# Demo: record a 20 s practice duel on arena_tiers, replay it: the re-simulation must reproduce the state hash.
"$T/$PROFILE/lgduel" --headless --no-settings --map arena_tiers --record artifacts/practice.lgdemo --secs 20 | grep -E "DEMO RECORDED"
"$T/$PROFILE/lgduel" --headless --no-settings --play-demo artifacts/practice.lgdemo --size 1280x720 --screenshot artifacts/demo_end.png | grep -E "DEMO (REPLAY|CHECK)"
# Textured theme: renders with many colours and the usual enemy / HUD / beam checks.
"$T/$PROFILE/lgduel" --headless --frames 3 --theme grid --map arena_tiers --size 1280x720 --screenshot artifacts/theme_grid.png | grep -E "CHECK"
# MSAA 4x: edges get blended, so the picture has many more distinct colours than the aliased one (12), and the usual checks still pass.
msaa_out=$("$T/$PROFILE/lgduel" --headless --no-settings --frames 3 --msaa 4 --size 1280x720 --screenshot artifacts/msaa4.png)
echo "$msaa_out" | grep -E "RENDER CHECK"
msaa_colors=$(echo "$msaa_out" | sed -n 's/.*distinct colours \([0-9]*\),.*/\1/p')
[ "${msaa_colors:-0}" -gt 30 ] || { echo "MSAA had no visible effect ($msaa_colors colours)"; exit 1; }
# Scoreboard (TAB): panel drawn high on the screen, enemy / crosshair / beam still visible.
"$T/$PROFILE/lgduel" --headless --no-settings --frames 20 --scoreboard-demo --size 1280x720 --screenshot artifacts/scoreboard.png | grep -E "scoreboard|CHECK"
# A theme loaded from a folder (PNG textures).
"$T/$PROFILE/lgduel" --headless --frames 3 --theme example --theme-dir assets/themes --map arena_pillars --size 1280x720 --screenshot artifacts/theme_example.png | grep -E "CHECK"
# Humanoid enemy model (forced colours), and the same model in its own skin colours.
"$T/$PROFILE/lgduel" --headless --frames 3 --enemy-model humanoid --size 1280x720 --screenshot artifacts/model_humanoid.png | grep -E "CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --enemy-model humanoid --skins --enemy-distance 110 --size 1280x720 --screenshot artifacts/model_skin.png | grep -E "CHECK"
# Environment report (also what a bug report should include).
"$T/$PROFILE/lgduel" --doctor | grep -E "^(adapter|audio|session):"
# Scaling: menu at 1.5x, and HUD text scaled with a 1080p window.
"$T/$PROFILE/lgduel" --headless --frames 3 --menu pause --ui-scale 1.5 --size 1280x720 --screenshot artifacts/menu_scaled.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --auto-hud --size 1920x1080 --screenshot artifacts/hud_auto.png | grep -E "CHECK"
# Every crosshair shape (with and without outline) passes the crosshair pixel check.
for x in cross dot circle tshape; do
  "$T/$PROFILE/lgduel" --headless --frames 3 --crosshair "$x" --crosshair-outline --no-sparks --size 1280x720 --screenshot "artifacts/crosshair_$x.png" | grep -E "CHECK"
done
# The other built-in themes (outlines, gradient sky).
for t in midnight paper ember; do
  "$T/$PROFILE/lgduel" --headless --frames 3 --theme "$t" --map arena_tiers --size 1280x720 --screenshot "artifacts/theme_$t.png" | grep -E "CHECK"
done
# KovaaK's import against the fake install in testdata (never the real one): report, theme
# import into a throwaway settings folder, then render the imported theme with its enemy colours.
rm -rf artifacts/kv && mkdir -p artifacts/kv
KV="--kovaaks-dir crates/client/testdata/kovaaks --settings artifacts/kv/settings.toml"
"$T/$PROFILE/lgduel" $KV --kovaaks-report | grep -E "^KOVAAKS found .*themes 2 \(problems 1\)"
"$T/$PROFILE/lgduel" $KV --import-kovaaks-theme current | grep -E "KOVAAKS THEME imported kovaaks-lgduel-test-full"
"$T/$PROFILE/lgduel" --settings artifacts/kv/settings.toml --headless --frames 3 --size 1280x720 --screenshot artifacts/kovaaks_theme.png | grep -E "CHECK"
# Every built-in map renders with the duel checks passing.
for m in arena_pillars arena_tiers; do
  "$T/$PROFILE/lgduel" --headless --frames 3 --map "$m" --size 1280x720 --screenshot "artifacts/map_$m.png" | grep -E "^MAP|CHECK"
done
# egui pause menu (pixel font) over the scene.
"$T/$PROFILE/lgduel" --headless --frames 3 --menu pause --size 1280x720 --screenshot artifacts/menu.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu pause-online --size 1280x720 --screenshot artifacts/menu_online.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu share --size 1280x720 --screenshot artifacts/menu_share.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu console --size 1280x720 --screenshot artifacts/console.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu settings --size 1280x720 --screenshot artifacts/settings.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu main --size 1280x720 --screenshot artifacts/mainmenu.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu online --size 1280x720 --screenshot artifacts/online_menu.png | grep -E "IMAGE menu|MENU CHECK"
"$T/$PROFILE/lgduel" --headless --frames 3 --menu host --size 1280x720 --screenshot artifacts/host_menu.png | grep -E "IMAGE menu|MENU CHECK"

printf '\nCHECK GREEN in %ss\n' "$(( $(date +%s) - start ))"

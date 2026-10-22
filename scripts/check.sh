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
for bin in lgduel lgduel-server lgduel-bot lgduel-tools; do
    "target/$PROFILE/$bin" >/dev/null
    echo "ok: $bin"
done

stage "headless match sim"
echo "SKIP: not implemented yet (PLAN Phase 4: networking / headless match)"

stage "netem matrix"
echo "SKIP: not implemented yet (PLAN Phase 5)"

stage "p2p rendezvous"
echo "SKIP: not implemented yet (PLAN Phase 7)"

stage "client offscreen render"
echo "SKIP: not implemented yet (PLAN Phase 3)"

printf '\nCHECK GREEN in %ss\n' "$(( $(date +%s) - start ))"

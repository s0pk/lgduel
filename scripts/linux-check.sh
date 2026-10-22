#!/usr/bin/env bash
# Run scripts/check.sh inside a Linux container (needs Docker). The checkout is
# mounted at /src; Linux build output goes to a named volume, not ./target.
set -euo pipefail
cd "$(dirname "$0")/.."
docker build -q -t lgduel-check -f ci/check.Dockerfile ci >/dev/null
MSYS_NO_PATHCONV=1 docker run --rm \
    -v "$(pwd -W 2>/dev/null || pwd):/src" \
    -v lgduel-cargo-registry:/usr/local/cargo/registry \
    -v lgduel-linux-target:/linux-target \
    -e CARGO_TARGET_DIR=/linux-target \
    -e MATCH_SECS="${MATCH_SECS:-60}" \
    lgduel-check ./scripts/check.sh

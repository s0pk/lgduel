#!/usr/bin/env bash
# Sanity-check packaging/arch/PKGBUILD without an Arch system: syntax, required
# fields, version equals the workspace version, every installed file exists.
set -euo pipefail
cd "$(dirname "$0")/.."
f=packaging/arch/PKGBUILD
bash -n "$f"
# shellcheck disable=SC1090
( source "$f"
  for v in pkgbase pkgname pkgver pkgrel pkgdesc arch url license makedepends source sha256sums; do
    [ -n "${!v:-}" ] || { echo "PKGBUILD: $v is empty"; exit 1; }
  done
  declare -F build package_lgduel package_lgduel-server >/dev/null || { echo "PKGBUILD: missing functions"; exit 1; }
  want=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
  [ "$pkgver" = "$want" ] || { echo "PKGBUILD pkgver $pkgver != Cargo.toml version $want"; exit 1; }
  echo "pkgver $pkgver ok" )
# Files the package installs from the source tree must exist.
for p in packaging/lgduel.desktop packaging/arch/lgduel-server.service packaging/arch/lgduel-relay.service \
         packaging/arch/lgduel.sysusers assets/icons/lgduel-48.png assets/icons/lgduel-128.png assets/icons/lgduel-256.png \
         assets/themes/example/theme.toml config/server.example.toml LICENSE README.md PLAY.md THEMES.md MAPS.md CONFIG.md SHARING.md KOVAAKS.md \
         HOSTING.md INSTALL.md DEPLOY.md; do
  [ -e "$p" ] || { echo "missing $p"; exit 1; }
done
grep -q '^ExecStart=/usr/bin/lgduel-server' packaging/arch/lgduel-server.service
grep -q '^ExecStart=/usr/bin/lgduel-server relay' packaging/arch/lgduel-relay.service
grep -q '^u lgduel ' packaging/arch/lgduel.sysusers
echo "PKGBUILD OK"

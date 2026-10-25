# Arch Linux packaging

`PKGBUILD` builds two packages from one source: **lgduel** (game + practice bot, desktop entry, icons, example theme) and
**lgduel-server** (dedicated server, join-code relay, systemd units, `sysusers.d` entry, `/etc/lgduel/server.toml`).
It builds from the GitHub source archive of tag `v<pkgver>` with the system `rust` (the `RUSTUP_TOOLCHAIN=stable` lines make
a rustup install ignore the `rust-toolchain.toml` pin).

## Test it locally (any Arch system or WSL Arch)
```bash
git archive --prefix=lgduel-0.2.2/ -o /tmp/lgduel-0.2.2.tar.gz HEAD
mkdir /tmp/pkg && cp packaging/arch/PKGBUILD /tmp/pkg/ && cd /tmp/pkg
sed -i "s|^source=.*|source=(\"lgduel-0.2.2.tar.gz::file:///tmp/lgduel-0.2.2.tar.gz\")|" PKGBUILD
makepkg -sf --noconfirm        # as a normal user (not root)
namcap *.pkg.tar.zst           # optional: lint the result
```

## Publish to the AUR (needs your AUR account and SSH key)
1. Push the release tag `v0.2.2` to GitHub (GitHub serves the source archive for any tag).
2. `git clone ssh://aur@aur.archlinux.org/lgduel.git aur-lgduel && cd aur-lgduel`
3. Copy `PKGBUILD` here, run `updpkgsums` (fills `sha256sums`), then `makepkg --printsrcinfo > .SRCINFO`.
4. `makepkg -si` once to test, then `git add PKGBUILD .SRCINFO && git commit -m "lgduel 0.2.2" && git push`.

On each new release: bump `pkgver` (and `Cargo.toml` `version`: `scripts/check-pkgbuild.sh` enforces they match), reset `pkgrel=1`,
`updpkgsums`, regenerate `.SRCINFO`, push.

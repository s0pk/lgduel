# LGDuel — handoff (2026-10-23)

This file is for the next person or model working on LGDuel. **Part 1** covers the repo and program: how to
build it, how it is laid out, and the rules not to break. **Part 2** is a step-by-step plan for the next
feature: **importing KovaaK's themes and hit sounds from a local KovaaK's install**.

Read Part 1 §1.4 (ground rules) before changing any code.

---

# Part 1 — The repo and the program

## 1.1 What LGDuel is

LGDuel is a standalone Lightning Gun duel trainer in the style of Quake Live, written in Rust. Players
practise LG tracking against bots offline, or duel online on dedicated servers, via peer-to-peer join codes
or through a relay. The target audience is aim-trainer players (KovaaK's / Aim Lab users) and Quake players.

- Deterministic 125 Hz simulation, Quake-style movement (strafe jumping, ramps, stairs) and the LG with
  lifesteal ("vamp" duels).
- Netcode: client prediction, interpolation, lag compensation, snapshots, a relay and a rendezvous server
  for join codes. Protocol **v9**.
- Rendering: wgpu with a flat-shaded "classic" look (the default, must stay pixel-identical), optional
  textured map themes, edge outlines, sky gradients and MSAA.
- Customisation: enemy forced colours (three bands), two enemy models (classic boxes and humanoid),
  crosshair shapes, HUD scale, menu accent colour and synthesised sounds. Looks are shared with
  `LGP1-…` profile codes and `.lgtheme` theme packs.
- Platforms: Windows (main dev PC) and native Linux (verified on Arch in WSL2). There is an AUR-style
  PKGBUILD in `packaging/arch/`.

## 1.2 Getting started on a new PC

```bash
git clone https://github.com/s0pk/lgduel.git
cd lgduel
# rustup installs the pinned toolchain (rust-toolchain.toml: 1.99.1 + rustfmt + clippy) on first use.
cargo build --release
./target/release/lgduel --practice          # play offline vs a bot (Windows: lgduel.exe)
./scripts/check.sh                          # full gate, ~2 min, must end with "CHECK GREEN"
```

- On Windows, run the scripts from **Git Bash**, not cmd or PowerShell.
- `check.sh` needs a GPU adapter for the offscreen render checks. It falls back to a software adapter
  if needed (KI-7).
- Linux build dependencies: see `INSTALL.md`. Arch: `base-devel rust alsa-lib vulkan-icd-loader libxkbcommon
  wayland libx11 libxcursor libxi libxrandr` plus your GPU's Vulkan driver. Debian/Ubuntu: `build-essential
  pkg-config libasound2-dev libvulkan1 mesa-vulkan-drivers`.
- Ignored by git: `target/`, `artifacts/*` (screenshots and logs from check.sh) and `dist/`.
- **GitHub Actions:** `.github/workflows/{ci,release}.yml` exist, but the GitHub account has a billing lock
  and every job is refused. The owner chose **not** to build releases in Actions: just push the repo. Do
  not spend time on CI.
- Branches: `main` and `v0.2-dev` both point at the same commit (`4ba9ae0` plus this handoff). Work on
  `main` unless told otherwise.

## 1.3 Repo map

| Path | What |
|---|---|
| `crates/sim` | Deterministic game simulation: no I/O, no clocks. `world.rs` (map TOML, `content_hash`), `pmove.rs` (movement), `game.rs` (players, LG, stats, `GameEvent`), `brush.rs`, `trace.rs`, `config.rs` |
| `crates/net` | Protocol, snapshots, prediction, interpolation, lag comp, `transport.rs`, `conn.rs`, `client.rs`, `rendezvous.rs`, `demo.rs` |
| `crates/server` | Headless dedicated server, relay, host-from-client, admin |
| `crates/bot` | Headless bot client and strafe patterns |
| `crates/tools` | netem, match runner, benchmarks |
| `crates/client` | The game: see below |
| `maps/` | `arena_flat`, `arena_pillars`, `arena_tiers` (TOML; `material = "…"` tags are visual only) |
| `assets/` | `themes/example/` (CC0 PNG theme), `icons/` |
| `packaging/` | `arch/PKGBUILD` (split: lgduel + lgduel-server), systemd units, sysusers, `lgduel.desktop` |
| `scripts/` | `check.sh` (the gate), `package.sh`, `check-pkgbuild.sh`, `linux-check.sh` |
| `HANDOFF.md` | **Start here**: repo overview, rules, and the next feature plan (KovaaK's import) |
| `STATE/` | `PLAN.md` (milestones + ticks), `PROGRESS.md` (log), `DECISIONS.md` (D1–D64), `KNOWN_ISSUES.md`, older handoff plans |
| Docs | `README.md`, `PLAY.md` (keys, menus), `CONFIG.md` (every setting), `THEMES.md`, `SHARING.md`, `MAPS.md`, `INSTALL.md`, `HOSTING.md`, `DEPLOY.md`, `docs/PERF.md` |

`crates/client/src` (about 15k lines):

| File | Role |
|---|---|
| `main.rs` | CLI parsing (`Options`, `parse_args`), the `App` (winit `ApplicationHandler`), headless modes (`run_headless`, `run_headless_online`, `run_headless_demo`) with **pixel checks**, `apply_settings`, `leave_session`/`end_session` |
| `render.rs` | wgpu renderer: flat pipeline (dynamic + HUD), textured world pipeline (2D texture array), sky quad, edge outlines, `Targets` (depth + MSAA), `push_box`, `push_obox`, `world_mesh_textured` |
| `textures.rs` | PNG decode with limits, resize to 256², mip chain, `TextureSet` (layer 0 = white, at least 2 layers for GL), `safe_relative` path check |
| `theme.rs` | `Theme` / `Material` / `Defaults` / `Edges` / `MapOverride` (serde TOML), built-ins `classic grid midnight paper ember`, `ThemeStyle::{builtin,load}`, `search_dirs`, `find_theme_dir`, `available`, `is_valid_theme_name`, `procedural(name)` textures `tiles concrete panel` |
| `themepack.rs` | `.lgtheme` export/import (safe unpack), `name_from_file` |
| `profile.rs` | `LGP1-` share codes (zlib + CRC32 + base64url), `is_look_key` whitelist, presets, `install_theme` |
| `settings.rs` | `Settings` (TOML at `%APPDATA%\LGDuel\settings.toml` or `~/.config/lgduel/settings.toml`), every struct `#[serde(default)]`, `sanitize()` |
| `console.rs` | Every settings field is automatically a cvar (`video.fov 100`). `ConsoleAction` holds commands (`preset`, `exportprofile`, `importtheme`, `reloadtheme`, `record`, …) |
| `menu.rs` | egui menus. Settings tabs `video_tab`, `visuals_tab`, `audio_tab`, `share_tab`, …; `color_row`, `row` helpers |
| `ui.rs` | egui integration, `apply_style`, `MENU_PANEL` const (used by the headless menu pixel check) |
| `hud.rs` | Pixel-font HUD, crosshair shapes, damage numbers, net graph, input display, `scoreboard_lines`/`scoreboard` (TAB) |
| `audio.rs` | cpal output, `Sound` enum (`Hit Kill FightWon FightLost UiClick Challenge`), `synth(sound, rate)`, `mix`, `Audio::{open, play, play_hit}` |
| `scene.rs` | `FrameView`, `scene_mesh` (players, beams, gun), `hud_mesh`, `track_hits`, `view_camera` |
| `models.rs`, `players.rs`, `effects.rs` | Enemy models (classic / humanoid), forced colours `EnemyColors {head, upper, lower}`, beam and sparks |
| `local.rs`, `net_session.rs`, `demo_play.rs`, `browser.rs`, `fight.rs`, `offscreen.rs`, `gfx.rs`, `icon.rs`, `font.rs`, `ttf.rs` | Offline session, online session, demo playback, server browser, end-of-fight summary, headless GPU, window surface, icons, fonts |

## 1.4 Ground rules (do not break these)

1. **One milestone = one commit**, with `./scripts/check.sh` ending in `CHECK GREEN` before committing.
   It runs fmt, clippy `-D warnings`, the build, all tests, the determinism golden test, headless matches,
   the netem matrix, p2p and many offscreen renders with pixel checks. **Never weaken or delete a test or
   check** to make it pass. Commit messages end with your model's `Co-Authored-By` line.
2. **Keep STATE up to date:** tick `STATE/PLAN.md`, add a `STATE/PROGRESS.md` entry saying what changed and
   what was verified, and add a `STATE/DECISIONS.md` entry (D65+) for each design choice.
3. **The default look is pixel-identical.** With no new settings, every existing render check must pass
   unchanged. New features get their *own* extra check runs.
4. **Leave the simulation alone.** The golden hash `0x9e74a8ea7918ee78` (`cargo test -p lgduel-sim --test
   determinism`) and `World::content_hash` of every built-in map must not change. Visuals never enter the sim.
5. **Visibility first.** No fog, no lighting that darkens players, no screen shake. Forced enemy colours
   are on by default. Models stay inside the hitbox. TAB scoreboard and other overlays must not cover the
   crosshair or enemy.
6. **Legal (D4):** only original or CC0 assets go in the repo. **Never commit anything from a KovaaK's
   install** or a community theme. Reading the user's *own* files from their install at runtime is fine.
7. **Hostile input:** every file the user loads (themes, packs, codes, PNGs, and soon KovaaK's files) has size
   caps, never panics, and reports problems as console lines. Tests cover truncated and garbage input.
8. **Performance:** at least 1000 fps at 1920×1080. Measure with
   `target/release/lgduel --no-settings --practice --synthetic-input --frames 4000 --size 1920x1080` and read
   the `FRAMESTATS` line (currently about 4000–6800 fps).

## 1.5 How the main pieces work (short)

- **Settings, cvars and the menu.** Add a field to a settings struct with a `Default`. It is then saved,
  becomes a console variable and can be shown in the menu with `row`/`color_row`. Clamp bad values in
  `Settings::sanitize`. CLI flags are parsed in `main.rs` `parse_args` (a big `match`). Headless test flags
  live there too.
- **Themes.** A theme folder is `<dir>/<name>/theme.toml` plus optional PNGs. Search order
  (`theme::search_dirs`): `--theme-dir`, then `<settings dir>/themes`, then `<exe dir>/themes`, then XDG dirs
  on Linux. A surface's material is resolved by map tag, then orientation (`floor`/`wall`/`ceiling`/`ramp`),
  then the per-map override, then falls back to classic. Colours are **linear** RGB 0..1. `Material.scale`
  is world units per texture repeat (default 128). Selecting a theme sets `video.theme`. The app reloads it
  via `refresh_style` (console `reloadtheme`).
- **Audio.** `Audio::open(volume)` builds a cpal stream at the device's rate and synthesises every `Sound`
  into `Vec<f32>` (mono, at the device rate). The game thread sends `Play {sound, gain, pitch}` over a
  channel, and the audio thread mixes voices (`mix`, linear interpolation, clamp). Audio is reopened in
  `apply_settings` when needed. Sounds are triggered in `main.rs` around `scene::track_hits` (hit),
  `GameEvent::Kill` (kill), fight end, and menu clicks.
- **Headless checks.** `lgduel --headless --frames N --size WxH --screenshot artifacts/x.png [flags]` renders
  offscreen. `run_headless` then counts exact colours: floor colour, the three enemy band colours (≥ 50 px,
  ≥ 20 for humanoid), crosshair, HUD text, beam, sparks and damage numbers. It prints `RENDER CHECK OK` or
  fails. `check.sh` greps those lines. Use `--no-settings` or `--settings <file>` to isolate from your
  personal settings.
- **Share codes.** `profile::is_look_key` decides which cvars go into an `LGP1-` code. It includes all
  `audio.*` except `audio.volume`. **File-path settings must be excluded** (see K4).

## 1.6 Status

- Done: v0.2 plan, visual customisation T1–T6 (`STATE/HANDOFF_VISUALS.md`), and Linux/look/share A1–A5,
  B1–B4, C1–C8 (`STATE/HANDOFF_LINUX_LOOK_SHARE.md`). The latest check.sh run was **GREEN on Windows and
  on Arch Linux (WSL2)**. `makepkg` builds and installs both packages.
- Blocked on the owner: the GitHub billing lock (CI), Docker Desktop crashing (the Docker deploy check),
  the VPS address for an official server, the v0.2.0 tag/release, and AUR submission.
- Open known issues worth knowing: KI-4 (physics constants not checked against QL), KI-6 (no player-player
  collision), KI-8 (few depth cues on flat floors), KI-12 (bots don't navigate), KI-13 (spectator can't pick
  an arena). See `STATE/KNOWN_ISSUES.md`.

## 1.7 Gotchas already hit

- **wgpu 30:** `depth_write_enabled: Some(..)`, `entry_point: Some("vs")`, `bind_group_layouts: &[Some(&bgl)]`,
  `multiview_mask: None`. Integer vertex attributes need `@interpolate(flat)`. **The GL backend rejects a
  one-layer texture array.**
- **egui 0.37:** use `Id::unique` (not `Id::new`), `ComboBox::from_id_salt`, and no `TopBottomPanel`.
- The HUD pixel font has uppercase letters, digits and some punctuation only. Upper-case HUD strings.
- Clippy: functions with many params use `#[allow(clippy::too_many_arguments)]`.
- Editing from Git Bash: heredocs mangle backslashes and `\1`. Write Python/Rust edits to a file with
  an editor tool instead of inline heredocs.
- `.gitattributes`: `* text=auto eol=lf`, PNGs binary, `crates/client/testdata/** -text` (exact bytes).
- Scripts need the executable bit in git (`git update-index --chmod=+x`), or Linux says "Permission denied".

---

# Part 2 — KovaaK's import: themes and hit sounds

**Goal.** A player who has KovaaK's installed can, with one click (or one console command), get:
1. their **KovaaK's themes** as LGDuel map themes (wall/floor/ceiling/ramp colours, a matching
   built-in texture, sky colour) and, optionally, their **enemy colours**;
2. their **KovaaK's hit sound** (and kill sound) playing in LGDuel instead of the synthesised one.

Optional stretch goal (K6): their KovaaK's crosshair image.

Everything is client-side. There is no protocol, server or sim change.

## K0 — Facts about KovaaK's files (verified on a real install, 2026-10-23)

These facts were checked on the dev PC (`E:\SteamLibrary\steamapps\common\FPSAimTrainer`, 116 themes, 432
sounds, 325 crosshairs). **Do not copy any of those files into the repo.** Use
`crates/client/testdata/kovaaks/` (original fixtures, CC0, see its README.txt).

**Install location.** Steam app id **824270**. The install root is
`<Steam library>/steamapps/common/FPSAimTrainer/`. User data lives **inside the install folder**, not in
AppData. The same holds under Proton on Linux, because the library path is a normal Linux path.

```
FPSAimTrainer/                      <- install root ("common/FPSAimTrainer")
  FPSAimTrainer.exe
  FPSAimTrainer/                    <- game dir
    sounds/          *.ogg (426) and *.wav (6). A sound's name = file stem ("apple_4-filter_2")
    crosshairs/      *.png (RGBA or RGB, 10x10 to 128x128, mostly 50x50)
    Content/Paks/    FPSAimTrainer-WindowsNoEditor.pak  <- built-in sounds/textures. NEVER read or extract.
    Saved/SaveGames/
      Themes/<anything>.json        one theme per file (116 here)
      PrimaryUserSettings.json      selected theme, kill sound, volumes, colours
      weaponsettings.ini            hit sounds, crosshair (UTF-8 BOM, CRLF)
      Weapons/*.wep, Scenarios/...  per-weapon/scenario overrides (ignore for now)
```

Steam libraries are listed in `<Steam root>/steamapps/libraryfolders.vdf` (Valve KeyValues text). The dev PC
has two: `C:\Program Files (x86)\Steam` and `E:\SteamLibrary`. Each entry has `"path"` and an `"apps"` block
whose keys are app ids, so look for `"824270"`. Paths in the file escape backslashes (`E:\\SteamLibrary`).

**Theme JSON** (`Saved/SaveGames/Themes/*.json`, tab-indented, UTF-8, sometimes with a BOM, so strip `\u{feff}`).
Key survey over all 116 files (count = files that have the key):

| Keys | In | Meaning / use |
|---|---|---|
| `themeName` | 116 | Display name; may contain `!`, spaces, mixed case |
| `{wall,floor}{Material,Roughness,Metallic,FullBright,Tint}` | 116 | Surfaces |
| `{wall,floor}TextureScale` | 103 | Optional; default 1 |
| `{ceiling,ramp}{Material,Roughness,Metallic,FullBright,Tint,TextureScale}` | 98 | Optional; when missing, use the floor's values (ramp) or the wall's (ceiling) |
| `*Tint` | — | `{"x","y","z"}` floats 0..1 = **linear** RGB (UE `FLinearColor`; max seen 1.0) |
| `overrideEnemyHeadColor`, `overrideEnemyBodyColor` | 116 | bool |
| `enemyHeadColor`, `enemyBodyColor` | 116 | `{"x","y","z"}` linear |
| `enemy{Head,Body}ColorOn{Hit,LookAt}`, `changeEnemyColorOn{Hit,LookAt}` | 103 | Ignore (LGDuel has no hit flash) |
| `skyColor` | 116 | `{"r","g","b","a"}` **0..255 sRGB bytes** (UE `FColor`) |
| `solidSkyColor` | 116 | bool; false = a skybox preset (`skyPresetId` 0–11) that we cannot reproduce |
| `enemyOutline*` | 1 | Ignore |
| roughness, metallic, glow, `teamGlow*`, `cloudCoverId`, `sunVisible`, `enemyColor{Roughness,Metallic,FullBright}` | — | Ignore (LGDuel has no lighting) |

`*Material` values seen (count): `MARBLE POLISHED` 148, `PURE COLOR` 54, `WHITE WOOD BOARD` 38, *(missing)* 36,
`GREY WOOD BOARD` 25, `BIANCA CARRARA MARBLE TILES` 21, `BRICK CLAY BEVELED` 18, `DRYWALL` 17,
`CONCRETE BASIC` 16, `CONCRETE POURED` 14, `GRID 8` 10, `GRID 16` 10, `CONCRETE TILES` 9, `BRICK GREY` 7,
`BRICK DARK` 5, `METAL STEEL` 4, `WOOD PLANK CLEAN` 4, `STONE TILES FACADE` 4, `PAINTED WALL A` 4, and others
seen once or twice (`GRID 64`, `SCIFI WALL`, `GROUND GRASS`, `OUTLAWS WALLPAPER`, `ROCK SLATE`, …).
The textures themselves live in the `.pak`, so we only map names to LGDuel's own procedural textures.

**PrimaryUserSettings.json.** Nested objects keyed with UE enum prefixes:
- `stringSettings."EStringSettingId::CurrentThemeName"`: the selected theme. Match it against `themeName`
  first, then the file stem, case-insensitively.
- `stringSettings."EStringSettingId::KillConfirmedSound"`: sound name (stem) or `"None"`/`""`.
- `floatSettings."EFloatSettingId::HitVolume"` (0..1), `"…::MasterVolume"`, `"…::SFXVolume"`, `"…::HitPitch"`.
- It also holds other user data. Read **only** these keys and never log the whole file.

**weaponsettings.ini.** Flat `Key=Value` lines (UTF-8 BOM, CRLF), the global weapon defaults:
- `BodyHitSound=apple_4-filter_2`: **the hit sound** (stem). `HeadHitSound=` is often empty. For LGDuel's
  single hit sound, use `BodyHitSound`, falling back to `HeadHitSound`.
- `CrosshairFile=aechSmall.png`, `CrosshairColor=X=0.901 Y=0.012 Z=0.012` (linear), `CrosshairScale=0.7`
  (used by K6 only).
- Some sound names refer to **built-in** sounds inside the `.pak` (`ShootSound=Shot` has no file in
  `sounds/`). If `sounds/<name>.ogg|.wav` doesn't exist, report "built into KovaaK's, can't be imported".

**Sound files** (432 checked): Ogg **Vorbis** at 44.1 kHz (372), 48 kHz (48), 22.05 kHz (5) and 88.2 kHz (1),
mono or stereo. WAV: 16-bit PCM (4), 32-bit (1, float or int) and **4-bit IMA ADPCM** (1, reject with a
clear message). Sizes: median 8.6 KB, max 4.4 MB (music). Hit sounds are short (< 0.5 s).

## K-design — decisions to record as D65

- **Copy what you import.** Converted themes go to `<settings dir>/themes/kovaaks-<slug>/theme.toml`, a
  normal LGDuel theme. The menu combo, `reloadtheme`, `.lgtheme` export and share codes then work
  unchanged. Imported sounds are copied (original bytes) into `<settings dir>/sounds/`. The user's own
  files stay on their own PC, which is legal. Imports keep working if KovaaK's moves or is uninstalled.
- **Never write into the KovaaK's folder.** Open its files read-only.
- **Approximate, documented mapping.** We cannot load KovaaK's textures (they are in the `.pak`), skyboxes
  or lighting. Colours are exact; texture style is the nearest of `tiles`/`concrete`/`panel`/none.
- **Visibility still wins.** Per-face shading (`shade = true`) stays on even for "full bright" KovaaK's
  themes, so walls stay distinguishable. Enemy colours are imported (the user's choice), but warn when an
  enemy colour is close to a wall or floor colour.
- **New dependencies:** `serde_json = "1"` (KovaaK's JSON) and `lewton = "0.10"` (pure-Rust Ogg Vorbis
  decoder, MIT/Apache-2.0) in `crates/client/Cargo.toml`. WAV is parsed by hand (small, no dependency).
  If `lewton` fails to build on the pinned toolchain, use `symphonia` with features `["ogg", "vorbis",
  "wav"]` (MPL-2.0) instead and note it in D65.
- **Share codes:** imported *themes* travel in `LGP1-` codes like any theme (the code embeds the
  theme.toml). Imported *sound files* do **not**: exclude the new `audio.hit_sound`/`audio.kill_sound_file`
  keys in `profile::is_look_key`, because a file name means nothing on another PC.

## K1 — Find the KovaaK's install (new `crates/client/src/kovaaks.rs`)

1. `pub struct Install { pub root: PathBuf }` (root = `…/common/FPSAimTrainer`), with helpers
   `game_dir()`, `sounds_dir()`, `crosshairs_dir()`, `save_dir()` (`game_dir()/Saved/SaveGames`),
   `themes_dir()`.
2. `pub fn is_install(root: &Path) -> bool`: true when `root/FPSAimTrainer/Saved/SaveGames` **or**
   `root/FPSAimTrainer/sounds` is a directory. Don't require the `.exe`.
3. `pub fn parse_library_folders(text: &str) -> Vec<(PathBuf, bool /*has 824270*/)>`: a tiny KeyValues
   tokenizer. Tokens are `"quoted strings"` (handle `\\` and `\"` escapes), `{` and `}`; skip `//` comments.
   Walk the tree: every object with a `"path"` value is a library, and it has KovaaK's if its `"apps"` child
   has the key `"824270"`. Don't panic on unbalanced braces; return what was parsed. Cap the input at 1 MB.
4. `pub fn steam_roots() -> Vec<PathBuf>`, the candidates that exist:
   - Windows: `%ProgramFiles(x86)%\Steam`, `%ProgramFiles%\Steam`, `C:\Program Files (x86)\Steam`. No
     registry crate needed. The manual path (step 5) covers unusual setups.
   - Linux: `$XDG_DATA_HOME/Steam` (default `~/.local/share/Steam`), `~/.steam/steam`, `~/.steam/root`,
     and `~/.var/app/com.valvesoftware.Steam/.local/share/Steam` (Flatpak).
   - macOS: none (KovaaK's is Windows-only).
5. `pub fn find(manual: Option<&Path>) -> Result<Install, String>`: if a manual path is given, accept it when
   `is_install(p)`, or when `p` is the inner game dir (`…/FPSAimTrainer/FPSAimTrainer`, then use its
   parent), or when `p/steamapps/common/FPSAimTrainer` is an install. Otherwise read each root's
   `steamapps/libraryfolders.vdf`. Try libraries that list 824270 first, then every library, then the
   root itself, checking `<lib>/steamapps/common/FPSAimTrainer`. The error message lists what was tried.
6. Settings: new section `[kovaaks]` with `path: Option<String>` (manual override; empty = auto). It becomes
   the cvar `kovaaks.path` automatically. CLI: `--kovaaks-dir PATH` (sets it for this run).
7. Tests: the VDF parser (two libraries, the app in the second, escaped backslashes, a comment, a
   truncated file); `is_install` and `find(manual)` on `crates/client/testdata/kovaaks` (root, inner dir,
   and a wrong dir that gives a clear error). Get the fixture path with
   `concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/kovaaks")`.

Done when: tests pass, `lgduel --kovaaks-report` (K5) on the dev PC prints the `E:\SteamLibrary\…`
install found through `libraryfolders.vdf`.

## K2 — Read and convert themes (in `kovaaks.rs`)

1. Serde model, **every field optional** so missing keys never fail:
   ```rust
   #[derive(Deserialize, Default)] #[serde(default, rename_all = "camelCase")]
   pub struct KvTheme {
       theme_name: Option<String>,
       wall_material: Option<String>, wall_tint: Option<Vec3f>, wall_texture_scale: Option<f32>, wall_full_bright: Option<f32>,
       floor_…, ceiling_…, ramp_…               // same four fields each
       override_enemy_head_color: bool, override_enemy_body_color: bool,
       enemy_head_color: Option<Vec3f>, enemy_body_color: Option<Vec3f>,
       solid_sky_color: bool, sky_color: Option<Rgba8>,
   }
   #[derive(Deserialize, Clone, Copy)] struct Vec3f { x: f32, y: f32, z: f32 }
   #[derive(Deserialize, Clone, Copy)] struct Rgba8 { r: u8, g: u8, b: u8, #[serde(default)] a: u8 }
   ```
   Unknown keys are ignored (no `deny_unknown_fields`: KovaaK's adds keys over time).
2. `pub fn list_themes(inst: &Install) -> (Vec<(String /*themeName or stem*/, PathBuf)>, Vec<String> /*problems*/)`:
   reads `*.json` in `themes_dir()` sorted by name, at most 2000 files and 256 KB each. Strip the BOM.
   Broken files go to `problems` as `"<file>: <error>"` and the rest still load. The fixture includes
   `broken.json`.
3. `pub fn convert_theme(kv: &KvTheme, fallback_name: &str) -> Converted` where
   `Converted { theme: theme::Theme, enemy: Option<EnemyColorsPatch>, warnings: Vec<String> }`:
   - `theme.name` = folder name (see step 4).
   - For each surface `floor wall ceiling ramp`, `materials.<surface> = Material { color: Some(tint), texture,
     scale, shade: true, tint_map_color: false }`. Defaults: ramp falls back to floor; ceiling falls back to
     wall; a missing tint uses classic (leave the material out). Clamp tints to 0..1; non-finite becomes 0.5.
   - `texture` from the material name (upper-case it, then the first match wins):

     | KovaaK's material contains | LGDuel texture |
     |---|---|
     | `PURE COLOR`, or the key is missing | none (flat colour) |
     | `GRID`, `TILE` | `tiles` |
     | `BRICK` | `tiles` |
     | `WOOD`, `PLANK`, `BOARD`, `METAL`, `SCIFI`, `PANEL`, `WALLPAPER`, `PAINTED` | `panel` |
     | `CONCRETE`, `DRYWALL`, `PLASTER`, `MARBLE`, `STONE`, `ROCK`, `COBBLE`, `GROUND`, `GRASS` | `concrete` |
     | anything else | `concrete` |

     Put the table in one `fn texture_for(material: &str) -> Option<&'static str>` with a table test.
   - `scale = 128.0 / kv_scale.clamp(0.25, 16.0)` (missing = 1.0). **Assumption:** a higher KovaaK's scale
     means more repeats. Check it visually with the GRID 8 / GRID 16 themes on the dev PC. If it is
     inverted, flip it in one `fn scale_for(kv: Option<f32>) -> f32` and record the result in D65.
   - Sky: `sky = srgb8_to_linear(sky_color)` (add `pub fn srgb8_to_linear(u8) -> f32` next to
     `offscreen::linear_to_srgb8`, with a round-trip test). When `solid_sky_color` is false, KovaaK's draws a
     skybox we can't copy. Use the same colour, with `sky_top = Some(sky * 0.6)` for a gentle gradient.
     A missing `skyColor` keeps the classic sky.
   - Edges stay off (KovaaK's has none).
   - Enemy patch: `head` = `enemyHeadColor` if `overrideEnemyHeadColor`; `upper` = `enemyBodyColor` if
     `overrideEnemyBodyColor`; `lower` = `upper × 0.75` (keeps LGDuel's three readable bands). Fields that
     aren't overridden stay `None`, meaning "keep the user's current colour".
   - Warnings: for each enemy colour in the patch, if its RGB distance to the floor or wall tint is < 0.25,
     add `"enemy <part> colour is close to the <surface> colour: enemies may be hard to see"`.
4. `pub fn theme_folder_name(name: &str) -> String`: `"kovaaks-"` + slug (lower-case ASCII alphanumerics
   kept; every other run of characters becomes `-`; trim `-`; at most 48 chars; empty gives `"theme"`).
   It must pass `theme::is_valid_theme_name`. Examples: `"!lobter air"` → `kovaaks-lobter-air`,
   `"3 AM"` → `kovaaks-3-am`.
5. `pub fn install_theme(conv: &Converted, themes_dir: &Path) -> Result<String /*name*/, String>`: write
   `themes_dir/<name>/theme.toml` (`toml::to_string(&conv.theme)`) plus an empty marker file
   `.kovaaks-import`. If the folder exists **without** the marker (a user's own theme), use `<name>-2`,
   `-3`, …. If it has the marker, overwrite it (re-import updates it). Write to a temp folder and rename it,
   like `themepack.rs`.
6. `pub fn import_themes(inst, which: Which, themes_dir) -> ImportReport` where `Which = Current | All |
   Named(String)`. `Current` reads `CurrentThemeName` (K0). `ImportReport { imported: Vec<String>, problems:
   Vec<String>, enemy: Option<EnemyColorsPatch> }`, where `enemy` comes from the current/named theme only
   (not from `All`).
7. Tests (fixture):
   - "LGDuel Test Full": wall texture `tiles` with tint (0.25, 0.3, 0.4), floor `concrete`, ceiling none,
     ramp `concrete`, sky = linear of (20, 30, 60), no `sky_top` (solid), and an enemy patch with all three
     colours.
   - "!lgduel minimal": a missing ceiling falls back to wall, a missing ramp falls back to floor, the scale
     default is applied, `sky_top` is set, and the head isn't overridden.
   - `broken.json` is reported and the other two still load.
   - The written `theme.toml` loads with `ThemeStyle::load` with **zero problems**.
   - The marker and collision behaviour works.
   - `theme_folder_name` table test.
   - The texture table test.
   - An enemy colour equal to the floor colour gives a warning.

## K3 — Decode sounds (new `crates/client/src/sound_file.rs`)

1. `pub struct Pcm { pub rate: u32, pub samples: Vec<f32> /* mono, -1..1 */ }`.
2. `pub fn decode(bytes: &[u8]) -> Result<Pcm, String>`, dispatching on magic bytes (`OggS` = Vorbis,
   `RIFF….WAVE` = WAV, anything else = "not an Ogg Vorbis or WAV file").
   - **WAV:** walk chunks with bounds checks, honouring odd-size padding. Read `fmt `: format tag 1 (PCM,
     8/16/24/32-bit), 3 (float32), or 0xFFFE (extensible; read the sub-format GUID's first 2 bytes as 1/3).
     Any other tag gives `"unsupported WAV encoding (tag 0x0011 = IMA ADPCM); re-save it as 16-bit PCM"`.
     Then read `data`. Channels 1..=8, rate 8000..=192000.
   - **Vorbis:** `lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes))`, then loop
     `read_dec_packet_itl()` collecting interleaved `i16` into f32 (`/ 32768.0`). Map lewton errors to
     strings. **Never unwrap.**
   - Downmix to mono by averaging channels.
3. Limits: input ≤ 8 MB. Keep at most **2.0 s** of decoded audio (stop decoding there, then apply a 20 ms
   linear fade-out). A peak below 1e-4 gives the error "sound is silent".
4. `pub fn resample(p: &Pcm, to_rate: u32) -> Vec<f32>`: linear interpolation (good enough for short
   effects). If the rates are equal, return a clone.
5. `pub fn load(path: &Path) -> Result<Pcm, String>`: read with the size cap, then `decode`. Errors are
   prefixed with the file name.
6. Tests (fixture `sounds/`): `lgduel_beep.ogg` (44.1 kHz mono, about 0.12 s, peak > 0.1);
   `lgduel_boop.wav` (48 kHz stereo 16-bit, mono after downmix, about 0.10 s); `lgduel_88k.ogg` decodes
   and resamples 88200 → 48000 with length ≈ ratio × input (±2); `lgduel_adpcm.wav` gives the ADPCM error
   text; `broken.ogg` gives an error and no panic. Also: hand-built WAVs in the test (8-bit, 24-bit,
   float32, an extensible header, a truncated `data` chunk, a chunk size larger than the file), a 3 s
   generated WAV trimmed to 2.0 s with the fade, and a silent WAV rejected. Every result is finite and
   within -1..1.

## K4 — Use custom sounds in the game

1. Settings (`settings.rs` `Audio`):
   - `hit_sound: Option<String>`: a **file name** inside `<settings dir>/sounds/` (None = synthesised).
   - `kill_sound_file: Option<String>`: the same for `Sound::Kill`.

   In `sanitize`, set a value to `None` if it isn't a plain file name (reuse the
   `textures::safe_relative` idea: no separators, no `..`). Exclude both keys in `profile::is_look_key`
   and add a test that they are not in a code.
2. `audio.rs`: change `Audio::open(volume)` to `Audio::open(volume, custom: &CustomSounds)` where
   `pub struct CustomSounds { pub hit: Option<sound_file::Pcm>, pub kill: Option<sound_file::Pcm> }`
   (`Default` = none). In `try_open`, for each `Sound`, use `resample(custom, device_rate)` if present,
   else `synth(sound, rate)`. Nothing changes in the mixer: samples are still at the device rate, and
   pitch still works through `step`.
3. **Voice cap:** a 0.4 s KovaaK's hit sound played on every LG hit (about 20/s) piles up. In the mixer,
   when a new `Hit` arrives and 4 `Hit` voices are already active, drop the oldest one. Add a `mix` test
   for the cap.
4. `main.rs`: a helper `fn custom_sounds(opts) -> (CustomSounds, Vec<String> /*problems*/)` loads the files
   from `<settings dir>/sounds/`. A load failure prints a console line and falls back to the synthesised
   sound. Never fail startup. Call it at all three `Audio::open` sites. In `apply_settings`, reopen audio when
   `hit_sound` or `kill_sound_file` changed (keep the last applied values in `App`).
5. `pub fn import_sounds(inst, sounds_dir_out: &Path) -> ImportReport` in `kovaaks.rs`:
   - Hit: `BodyHitSound`, falling back to `HeadHitSound`, from `weaponsettings.ini`. Parse lines with
     `split_once('=')`, strip the BOM, trim `\r`, and cap the file at 256 KB.
   - Kill: `KillConfirmedSound` from `PrimaryUserSettings.json`.
   - Resolve `<sounds_dir>/<stem>.ogg`, then `.wav`, matching the stem **case-insensitively** by scanning
     the folder (Linux is case-sensitive; KovaaK's names come from Windows). `""`/`"None"` gives "no sound
     set"; a missing file gives "built into KovaaK's, can't be imported".
   - `sound_file::load` it to validate it, then copy the **original bytes** to `<settings dir>/sounds/<file>`.
     Return the names to store in `audio.hit_sound` / `audio.kill_sound_file`, and set
     `audio.kill_sound = true` when a kill sound was imported.
   - Optional: copy `HitVolume × SFXVolume` into `audio.hit_volume` (clamped 0..1). Only do this when the
     user ticks "also copy volume".
6. Console: `hitsound` (prints the current one), `hitsound <file in sounds/>`, `hitsound default`.
7. Tests: `import_sounds` on the fixture (hit = `lgduel_beep.ogg`, kill = `lgduel_boop.wav`, both copied to
   a temp dir, the report contents); case-insensitive lookup; a built-in name (`Shot`); `CustomSounds`
   resampled to 48 kHz has the expected length.

## K5 — UI, console and CLI

1. **Share tab** (`menu.rs` `share_tab`), new section "KovaaK's":
   - A status line: `Found: E:\SteamLibrary\steamapps\common\FPSAimTrainer (116 themes, 432 sounds)` or
     `Not found` with a text field for the folder (bound to `kovaaks.path`) and a "Search again" button.
   - Theme: a combo of KovaaK's theme names (current one preselected), a "Use my enemy colours" checkbox
     (default on), and buttons **Import selected**, **Import current**, **Import all**.
   - Sounds: **Use my KovaaK's hit sound**, **Use my kill sound**, **Test** (plays the hit sound), and
     **Back to default sound**.
   - The result shows below it: imported names, warnings and problems (also printed to the console).
   - After a theme import, set `video.theme` to it and refresh themes (`self.menu.themes`, `refresh_style`)
     so it shows immediately. Apply the enemy patch to `s.enemy_colors` when the box is ticked and keep
     `models.forced_colors` as it is.
   - Scan the install once when the tab opens (and on "Search again"), not every frame. Keep the scan
     result in the menu state.
2. Add `Action::Kovaaks(KovaaksAction)` variants handled in `main.rs`, like the existing share actions.
3. Console: `kovaaks` (status), `kovaaks themes` (list), `kovaaks import <name>|current|all`,
   `kovaaks sounds` (import hit and kill sound).
4. CLI (for scripts and check.sh). These **exit** after doing their job, like `--doctor`. Note that
   `--import-theme` does *not* exit; it carries on into the game, so don't copy that one:
   - `--kovaaks-report`: prints
     `KOVAAKS found <root> | themes N (problems M) | sounds N | current "<name>" | hit "<stem>" | kill "<stem>"`
     or `KOVAAKS not found: <tried…>`, and exits 0 either way.
   - `--import-kovaaks-theme <name|current|all>`: prints `KOVAAKS THEME imported <folder>` per theme and
     sets `video.theme` to the current/named one, then saves settings.
   - `--import-kovaaks-sounds`: prints `KOVAAKS SOUND hit <file>` / `kill <file>`, then saves settings.
5. **check.sh** stage (after the theme renders), using the fixture and a throwaway settings file so it never
   touches the real one:
   ```bash
   # KovaaK's import (fake install in testdata): report, theme + sounds import, render with the imported theme.
   rm -rf artifacts/kv && mkdir -p artifacts/kv
   KV="--kovaaks-dir crates/client/testdata/kovaaks --settings artifacts/kv/settings.toml"
   "$T/$PROFILE/lgduel" $KV --kovaaks-report | grep -E "^KOVAAKS found .*themes 2 \(problems 1\)"
   "$T/$PROFILE/lgduel" $KV --import-kovaaks-theme current | grep -E "KOVAAKS THEME imported kovaaks-lgduel-test-full"
   "$T/$PROFILE/lgduel" $KV --import-kovaaks-sounds | grep -E "KOVAAKS SOUND hit lgduel_beep.ogg"
   "$T/$PROFILE/lgduel" --settings artifacts/kv/settings.toml --headless --frames 3 --size 1280x720 --screenshot artifacts/kovaaks_theme.png | grep -E "CHECK"
   ```
   In `run_headless`, a non-classic theme already skips the floor-colour check and requires ≥ 50 distinct
   colours. The imported enemy colours (orange/green) must still pass the enemy band checks. If they don't,
   check that the patch was applied, and don't lower the thresholds. Open the screenshot and look at it.

## K6 (optional) — KovaaK's crosshair image

LGDuel's HUD has no textured pipeline. Do it the pixel-font way:
- Add a `CrosshairShape::Image` variant and `hud.crosshair.image: Option<String>` (a file in
  `<settings dir>/crosshairs/`, copied on import from `CrosshairFile` in `weaponsettings.ini`).
- At load, decode the PNG with `textures.rs` (≤ 256×256, ≤ 1 MB). Turn pixels with alpha ≥ 128 into
  rectangles, **merging horizontal runs** of the same colour (a typical 50×50 crosshair becomes a few dozen
  quads). Cache this per file.
- Draw it centred at `scale = CrosshairScale` (KovaaK's 1.0 = image pixels). Tint it with
  `CrosshairColor` when the image is white or grey, otherwise keep the image colours.
- Tests: the fixture `crosshairs/lgduel_dot.png` (16×16 white disc of radius 3) gives quads covering
  exactly the opaque pixels. The default crosshair output is unchanged. check.sh renders with the imported
  crosshair and the "crosshair" pixel check still passes, using the imported colour.

## K7 — Docs and state

- New `KOVAAKS.md`: what is imported, what isn't (textures in the `.pak`, skyboxes, lighting, per-weapon or
  per-scenario sounds), where files are copied, how to point at a custom install, and "re-import to update".
  Link it from `README.md`, `SHARING.md` and `PLAY.md`.
- `CONFIG.md`: `kovaaks.path`, `audio.hit_sound`, `audio.kill_sound_file` (+ `hud.crosshair.image` if K6).
- `STATE/PLAN.md`: a "KovaaK's import" section with K1–K7 ticked as they land. `PROGRESS.md` entries.
  `DECISIONS.md` D65 (the K-design list above, plus the scale direction found in K2).
- Manual check on a PC with KovaaK's: import current, import all (expect about 116 themes and a few warnings
  for low-contrast enemy colours), hit sound on, play `--practice`, hear it, check the fps is still ≥ 1000.

## Order and size

K1 → K2 → K3 → K4 → K5 → K7, one commit each with check.sh green. K6 goes last, if wanted. Expect about
1200–1800 new lines, mostly in `kovaaks.rs`, `sound_file.rs` and their tests.

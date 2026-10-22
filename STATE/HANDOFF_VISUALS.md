# LGDuel — handoff plan: map themes, textures, better built-in character models

## Context
LGDuel (Rust, wgpu 30 + winit 0.30 + egui 0.37, repo `C:\Users\sopk\lgduels`) is feature-complete for v0.2
(see `STATE/PLAN.md`; remaining items there are blocked on the user: GitHub billing lock, Docker, VPS address).
The user wants **client-side visual customisation**: better character models and the ability to change how
models, textures and map theming look. This plan is written to be handed to a cheaper model, so every step
names files, types and checks, and keeps each milestone small enough to finish and verify on its own.

User decisions (2026-10-23):
- Character models: **built-in only** (made in code, no model file loading).
- Default look: **keep the classic flat look**; textured themes are opt-in.
- Enemy visibility: **forced colours ON by default**; a setting turns them off to show the model's own skin.

Everything here is **client-side only**: no change to the network protocol, server, simulation or bots.

---

## 0. Read first: ground rules (do not break these)

1. **Workflow per milestone**: implement → `cargo fmt --all` → `cargo clippy --workspace --all-targets --release -- -D warnings`
   → `./scripts/check.sh` (Git Bash, ~2 min, must end `CHECK GREEN`) → one git commit ending with
   `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (or your own model line) → tick the item in
   `STATE/PLAN.md`, append a short entry to `STATE/PROGRESS.md` (what changed + what was verified) and a
   `D<n>` entry to `STATE/DECISIONS.md` for any design choice. Never weaken or delete existing tests/checks.
2. **Default look must stay pixel-identical.** check.sh renders screenshots and counts exact colours
   (`run_headless` in `crates/client/src/main.rs` ~line 655: floor colour = `world.colors[0]`, enemy forced
   colours, crosshair, HUD text, beam, sparks, menu panel `[27,27,27]`). With theme `classic`, model
   `classic` and forced colours on, those checks must pass **unchanged**. New looks get their own extra
   check runs with their own assertions.
3. **Simulation untouched.** `World::content_hash()` (`crates/sim/src/world.rs`) must not change for any
   built-in map (it is sent in the server handshake); the determinism golden test
   (`cargo test -p lgduel-sim --test determinism`) must stay green. Visual data never goes into the hash.
4. **Visibility first** (spec §1): no fog, no lighting that darkens players, no screen effects; forced enemy
   colours default on; models must stay inside the hitbox (what you see is what you can hit).
5. **Legal** (DECISIONS D4): only original or procedurally generated assets (CC0/MIT). No Quake/id assets.
6. **Performance**: keep ≥ 1000 FPS at 1080p on the dev PC. Measure:
   `target/release/lgduel --no-settings --practice --synthetic-input --frames 4000 --size 1920x1080` → `FRAMESTATS` line
   (baseline ~4000 fps, see `docs/PERF.md`).

### Gotchas already hit in this codebase
- Shell is **Git Bash on Windows**. Edit code with the Edit/Write tools; heredocs mangle backslashes.
- **wgpu 30** API: `depth_write_enabled: Some(true)`, `depth_compare: Some(..)`, `entry_point: Some("vs")`,
  `bind_group_layouts: &[Some(&bgl)]`, `multiview_mask: None`, `cache: None`. When unsure, read the source in
  `~/.cargo/registry/src/*/wgpu-30*/`. Integer vertex attributes need `@interpolate(flat)` in WGSL.
- **egui 0.37**: `egui::Id::new` is deprecated (use `Id::unique`); panels need a parent `Ui` (use
  `egui::Area`/`Window`); combos are `egui::ComboBox::from_id_salt`.
- The HUD pixel font (`crates/client/src/font.rs`) has uppercase, digits and some punctuation only — no "…".
- clippy `-D warnings`: functions with many params use `#[allow(clippy::too_many_arguments)]` (existing pattern).
- Settings fields become console variables automatically (`crates/client/src/console.rs` walks the
  serialized `Settings`); every settings struct has `#[serde(default)]`, so new fields need a `Default`.

---

## Architecture overview (what exists, what gets added)

Today (`crates/client/src/render.rs`): one vertex type `Vertex { pos: [f32;3], color: [f32;3] }`, one flat
WGSL shader, three draws: static world buffer (`Renderer::set_world` → `world_mesh` → `push_box` /
`push_convex`, per-face brightness `FACE_SHADE`), dynamic buffer (players, beams, sparks, gun: built in
`scene::scene_mesh`), HUD buffer. Players: `players::push_player` draws three solid boxes (lower/upper/head
bands at `LOWER_TOP = 0.45`, `HEAD_BOTTOM = 0.85` of hull height) + a facing nub, coloured by
`EnemyColors`. Map colours come from map TOML `color` fields (`World.colors`, `ConvexBrush.color`); the sim
ignores colours.

Added by this plan:
- **World pipeline** (new): `WorldVertex { pos, uv, color, layer }` + a 2D texture array. Layer 0 is a 1×1-equivalent
  white texture, so an untextured surface renders exactly `color` (keeps the classic look identical).
  The flat pipeline stays for dynamic geometry and HUD.
- **Materials**: optional `material = "name"` tags on map entries (sim parses and stores, never hashes).
- **Themes** (`theme.rs`): sky colour + materials (texture, colour, scale) + defaults by surface
  orientation + per-map overrides. Built-in `classic` (current look) and `grid` (procedural textures); user
  themes from a folder.
- **Models** (`models.rs`): built-in `classic` (today's boxes, unchanged) and `humanoid` (procedural,
  animated by position), forced-colour banding, skins when forced colours are off.

---

## Milestones (do in order; each is one commit with check.sh green)

### T1 — Material tags in maps (sim, small)
Files: `crates/sim/src/world.rs`, `crates/sim/src/brush.rs`, World literal sites.
1. In `world.rs` add `#[serde(default)] material: Option<String>` to the private file structs `BrushFile`,
   `RampFile`, `StairsFile`, `ConvexFile` (they use `deny_unknown_fields`, so the key must be declared).
2. `World` gets `pub materials: Vec<Option<String>>`, parallel to `brushes`/`colors` (stairs steps copy their
   entry's tag). `ConvexBrush` (brush.rs) gets `pub material: Option<String>`, set to `None` in its
   constructors and assigned after construction in `from_toml_str` (same place colours are set).
3. Build with `cargo build --workspace --all-targets`; for every "missing field `materials`" error in a
   `World { .. }` literal add `materials: Vec::new()` (known sites: sim `game.rs`, `pmove.rs`, `trace.rs`
   tests, `sim/tests/{ramps,movement_golden,map_movement}.rs`, `server/src/lib.rs` tests,
   `server/tests/prediction.rs`, `bot/src/netbot.rs`, `client/src/{main,net_session}.rs` tests — fix what
   the compiler reports). Renderers must treat a missing entry as `None` (`materials.get(i).cloned().flatten()`).
4. **Do not** touch `World::content_hash`. Extend test `content_hash_tracks_geometry_not_colour`: set a material
   on a brush and a convex → hash unchanged. Add a parse test: `material = "trim"` on brush/ramp/stairs/convex
   is stored; omitted → `None`.
5. Document the key in `MAPS.md` ("visual only; themes decide what it looks like").
Done when: sim tests + determinism test + check.sh green.

### T2 — Textured world pipeline, classic look identical
Files: `crates/client/src/render.rs`, new `crates/client/src/textures.rs`, callers of `set_world` in `main.rs`.
1. `textures.rs`:
   - `pub const TEX: u32 = 256;` `pub struct Image { pub rgba: Vec<u8> }` (always TEX×TEX RGBA8 sRGB).
   - `pub fn white() -> Image`, `pub fn mip_chain(img: &Image) -> Vec<Vec<u8>>` (2×2 box filter down to 1×1; 9 levels).
   - `pub struct TextureSet { pub layers: Vec<Image>, pub index: HashMap<String, u32> }` with `new()` (layer 0 = white)
     and `add(name, Image) -> u32` (dedupe by name; cap 64 layers, extra → 0).
   - `pub struct GpuTextures { pub view: wgpu::TextureView, pub sampler: wgpu::Sampler }` and
     `pub fn upload(set: &TextureSet, device, queue) -> GpuTextures`: `TextureDimension::D2` with
     `depth_or_array_layers = layers.len()`, format `Rgba8UnormSrgb`, `mip_level_count = 9`,
     `queue.write_texture` per layer per mip, view with `TextureViewDimension::D2Array`, sampler
     address mode `Repeat`, mag/min `Linear`, mipmap `Linear`.
2. `render.rs`:
   - `#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)] pub struct WorldVertex { pub pos: [f32;3], pub uv: [f32;2], pub color: [f32;3], pub layer: u32 }`.
   - `WORLD_SHADER`: same uniform at group 0 binding 0; group 1: binding 0 `texture_2d_array<f32>`,
     binding 1 `sampler`. Fragment: `textureSample(tex, samp, i.uv, i.layer).rgb * i.color`
     (`layer` output with `@interpolate(flat)`). Separate pipeline `world_pipeline`, depth like the flat one.
   - Refactor geometry once: `pub fn box_faces(b: &Aabb) -> [([Vec3; 4], Vec3 /*normal*/, f32 /*shade*/); 6]`;
     make `push_box` use it (its test must still pass bit-for-bit). For convex: iterate `b.face(i)` as today.
   - `pub fn planar_uv(p: Vec3, n: Vec3, scale: f32) -> [f32; 2]`: dominant normal axis → |nz| max: `(x, y)`,
     |nx|: `(y, -z)`, |ny|: `(x, -z)`; each divided by `scale`.
   - `pub fn world_mesh_textured(w: &World, look: &dyn Fn(Surface) -> Look) -> Vec<WorldVertex>` where
     `Surface { material: Option<&str>, normal: Vec3, map_color: [f32;3], is_convex: bool }` and
     `Look { layer: u32, color: [f32;3] /* before face shade */, scale: f32, shade: bool }`. Vertex colour =
     `look.color × (FACE_SHADE / shade_for if look.shade else 1.0)`. Fan triangulation identical to today.
   - `Renderer`: hold `GpuTextures` + bind group; `set_world(device, queue, w, theme)` cache key =
     `(w.content_hash(), theme.cache_key())`; draw world with `world_pipeline`, then dynamic + HUD with the
     flat pipeline exactly as now. Until T3 exists, pass a stub "classic" look (layer 0, map colour, shade on).
3. Update every `set_world` caller (`main.rs`: App draw, `run_headless`, `run_headless_online`,
   `run_headless_demo`; grep `set_world(`).
4. Test (render.rs): for every built-in map, `world_mesh_textured` with the classic look yields the same
   positions and colours, in the same order, as the old `world_mesh` (keep `world_mesh` for this test).
Done when: check.sh green with **no** change to any pixel assertion (proves the look is identical) and FPS ≥ 1000.

### T3 — Theme system + built-in themes `classic` and `grid`
Files: new `crates/client/src/theme.rs`, `textures.rs` (procedural images), `settings.rs`, `menu.rs`, `main.rs`, `scripts/check.sh`.
1. `theme.rs` data (serde, all `#[serde(default)]`):
   ```toml
   name = "grid"
   sky = [0.02, 0.02, 0.03]          # clear colour (linear)
   [defaults]                        # material names by surface when the map has no tag
   floor = "floor"; wall = "wall"; ceiling = "ceiling"; ramp = "floor"
   [materials.floor]
   texture = "tiles"                 # built-in procedural name, or a PNG file (T4)
   color = [0.55, 0.56, 0.60]        # omit = use the map's colour
   tint_map_color = false            # true: multiply by the map colour
   scale = 128.0                     # world units per texture repeat
   shade = true                      # per-face brightness (FACE_SHADE)
   [maps.arena_tiers.materials.floor] # per-map overrides (same fields)
   ```
   Resolution per face (write as one function `resolve(&Theme, map_name, Surface) -> Look`, unit-tested):
   map tag → `maps[map].materials[tag]` → `materials[tag]`; no tag → orientation (`n.z > 0.7` floor,
   `n.z < -0.7` ceiling, else wall; convex top → `defaults.ramp`) → same lookup; anything missing → classic look
   (layer 0, map colour, shade on).
2. Built-ins (no files): `classic` = empty theme, sky = current `CLEAR` in main.rs (→ identical look).
   `grid` = procedural textures from `textures.rs`: `pub fn procedural(name: &str) -> Option<Image>` with
   `"tiles"` (light grid lines on mid grey), `"concrete"` (low-contrast value noise from a fixed integer hash,
   deterministic), `"panel"` (ceiling stripes). Keep textures mid-grey/low-contrast so the forced enemy colours
   stay the most saturated thing on screen.
3. Settings: `Video.theme: String` (default `"classic"`) → cvar `video.theme`; `--theme NAME` CLI; Visuals tab
   combo "Map theme" listing `theme::available()` (built-ins now, + folders in T4). Unknown name → fall back to
   classic and print a console line.
4. Clear colour: use `theme.sky` instead of the `CLEAR` const where the scene is drawn (keep `CLEAR` as the
   classic value).
5. check.sh: add
   `"$T/$PROFILE/lgduel" --headless --frames 3 --theme grid --map arena_tiers --size 1280x720 --screenshot artifacts/theme_grid.png | grep -E "CHECK"`.
   In `run_headless`, when the theme is not classic: skip the exact floor-colour check, require
   `distinct colours ≥ 50`; keep the enemy / HUD / beam checks as they are.
6. Tests: theme TOML parse, resolution order (tag, per-map override, orientation default, fallback),
   procedural images deterministic (hash of pixels fixed), classic theme ⇒ every `Look` has layer 0.
7. Inspect the screenshot visually (Read tool on the PNG): textures visible, enemy clearly visible.
Optionally tag some built-in map entries with materials (e.g. `trim` on cover walls) — allowed because tags
don't change the content hash (re-run the content-hash test).

### T4 — User themes from folders
Files: `theme.rs`, `textures.rs`, `settings.rs` (path helper), `scripts/package.sh`, new `assets/themes/example/`, `THEMES.md`.
1. Search paths, first match wins: `<settings dir>/themes/<name>/theme.toml` (settings dir = parent of
   `settings::default_path()`), then `<exe dir>/themes/<name>/theme.toml` (shipped example).
2. `texture = "file.png"`: resolve relative to the theme folder; reject absolute paths and `..` components;
   max 4 MB per file, max 2048×2048, max 32 textures per theme. Decode with the `png` crate (already a
   dependency: `png::Decoder::new(BufReader::new(file))`, `read_info`, `next_frame`), convert
   Grayscale/GrayAlpha/RGB/RGBA 8-bit to RGBA8, resize to 256×256 (box filter if larger, nearest if smaller).
3. Errors never crash: a bad material falls back to classic for that material; print each problem once to the
   console/log (`menu.console.print`), e.g. `theme example: floor.png: not found`.
4. Console command `reloadtheme` (add to `console.rs` as a `ConsoleAction`) re-reads the theme from disk and
   re-uploads (bump `cache_key`).
5. `lgduel-tools make-example-theme OUT_DIR`: writes `theme.toml` + 3 procedural PNGs (original, CC0) using
   `offscreen::save_png`-style encoding; run it once and commit the output to `assets/themes/example/`
   (+ a README stating CC0). `package.sh` and `.github/workflows/release.yml` copy it to `themes/example/`.
6. `THEMES.md`: folder layout, every key, resolution order, limits, how to make a texture tile.
7. Tests: load `assets/themes/example` (path from `env!("CARGO_MANIFEST_DIR")`); missing file → fallback +
   message; `../x.png` and absolute paths rejected; oversized dimensions rejected.
8. check.sh: render with `--theme example` (works because the exe-dir/repo search finds it — add the repo
   `assets/themes` as a third search path in debug/test builds, or pass `--theme-dir assets/themes`).

### T5 — Built-in character models + skins
Files: new `crates/client/src/models.rs`, `players.rs`, `scene.rs`, `effects.rs`/`render.rs` (shared box helper), `settings.rs`, `menu.rs`, `main.rs`, `scripts/check.sh`.
1. Extract the oriented-box code from `scene::push_gun` into a shared
   `pub fn push_obox(v: &mut Vec<Vertex>, centre: Vec3, axes: (Vec3, Vec3, Vec3), half: Vec3, color: [f32;3], shade: bool)`
   (in `render.rs`); make `push_gun` use it (its test `gun_model_options` must still pass: +72 vertices).
2. `models.rs`:
   - `#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)] #[serde(rename_all = "lowercase")] pub enum ModelKind { Classic, Humanoid }`.
   - `pub struct ModelPose { pub origin: Vec3, pub yaw: Angle, pub pitch: Angle, pub ducked: bool, pub firing: bool }`.
   - `pub struct Skin { pub head: [f32;3], pub torso: [f32;3], pub arms: [f32;3], pub legs: [f32;3] }` (serde, Default = neutral greys).
   - `pub fn push_model(v, kind, pose, phys: &PhysicsConfig, enemy: &EnemyColors, skin: &Skin, forced: bool)`:
     `Classic` → call existing `players::push_player` unchanged (forced colours as today; when `forced == false`
     colour bands with skin legs/torso/head instead).
   - `Humanoid`: legs ×2, torso, arms ×2 holding a short LG along `pitch`, head, built from `push_obox`
     parts sized as fractions of the hull (`player_mins/maxs`, crouch uses `crouch_maxs_z` and bends legs).
     Animation is **stateless**: leg swing phase = `(origin.x + origin.y) / 48.0` (moves when the player moves,
     still when standing); slight torso lean toward the facing direction. Rotate around the hull centre by `yaw`.
   - Forced colours: after building, recolour each triangle (3 vertices) solid by its centroid height band using
     `players::LOWER_TOP` / `HEAD_BOTTOM` (make them `pub`): below → `enemy.lower`, middle → `enemy.upper`,
     top → `enemy.head`; no face shading (same rule as classic).
3. Settings: new section `[models]`: `enemy = "classic"`, `forced_colors = true`, `skin = Skin`. Visuals tab:
   "Enemy model" combo, "Forced colours (recommended)" checkbox, skin colour pickers shown when unticked
   (reuse `color_row`). CLI `--enemy-model humanoid`.
4. `scene::scene_mesh`: replace `players::push_player(...)` with `models::push_model(...)`; pass the new
   settings in (add a parameter; update the callers in `main.rs` and the scene tests). Spectator and demo
   views use the same path.
5. Tests (`models.rs`): every Humanoid vertex lies inside the hitbox ± 1 unit (standing and crouched, several
   yaws/pitches, LG barrel included); with forced colours all three band colours are present and nothing else;
   the classic model output is byte-identical to `players::push_player`; animation phase changes vertex
   positions between two origins 24 units apart.
6. check.sh: render `--enemy-model humanoid --size 1280x720 --screenshot artifacts/model_humanoid.png`; in
   `run_headless` lower the per-band pixel minimum to 20 when the model is not classic (keep 50 for classic).
   Inspect the screenshot.

### T6 — Docs, settings UI pass, state
- `PLAY.md` (Settings → Visuals: map theme, enemy model, forced colours, skins), `CONFIG.md` (new keys:
  `video.theme`, `[models]`), `THEMES.md` linked from README, `docs/PERF.md` (FPS with `grid` + `humanoid`).
- Re-run the FPS measurement with `--theme grid --enemy-model humanoid`; record it.
- STATE files updated; PLAN.md gets a "Visual customisation" section listing T1–T6 ticked.

---

## Verification (every milestone, and end-to-end at the end)
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --release -- -D warnings`,
  `cargo test --workspace --release`, `./scripts/check.sh` → `CHECK GREEN`.
- Default look unchanged: check.sh's existing render/menu/online checks pass without edits to their thresholds.
- Sim untouched: `cargo test -p lgduel-sim --test determinism` (golden hash) and the content-hash test.
- Visual: open the new screenshots in `artifacts/` (`theme_grid.png`, `theme example`, `model_humanoid.png`)
  and confirm textures/models render and enemies are clearly visible.
- Live: `target/release/lgduel --practice --theme grid --enemy-model humanoid` → change theme / model / forced
  colours in Settings → Visuals and via console (`video.theme classic`, `models.forced_colors false`); look must
  update without restart (world re-upload on theme change).
- Performance: FRAMESTATS command above ≥ 1000 FPS avg at 1920×1080.

## Critical files
`crates/client/src/render.rs` (pipelines, `push_box`, `push_convex`, `world_mesh`, `Renderer::set_world/draw`),
`crates/client/src/players.rs` (classic model, bands, `EnemyColors`), `crates/client/src/scene.rs`
(`scene_mesh`, `push_gun`, `view_camera`), `crates/client/src/main.rs` (`run_headless` pixel checks, App draw,
`CLEAR`), `crates/client/src/settings.rs`, `crates/client/src/menu.rs` (`visuals_tab`, `video_tab`,
`color_row`), `crates/client/src/console.rs`, `crates/sim/src/world.rs` + `brush.rs` (map parsing, content
hash), `scripts/check.sh`, `scripts/package.sh`, docs (`MAPS.md`, `CONFIG.md`, `PLAY.md`).

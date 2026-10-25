# Map themes

A theme changes how the arena **looks** on your PC only: textures, colours and the sky. It never reaches the
server, never affects collision or spawns, and doesn't change what other players see. The default theme,
`classic`, is the original flat-shaded look; `grid`, `midnight` (dark blue, light outlines), `paper` (light grey, dark outlines) `ember` (dark red, orange outlines) and `quake` (dark stone blocks, riveted steel floor plates, tread-plate ramps; no outlines) are built-in textured themes.

Pick one in **Settings → Visuals → Map → Theme**, with `--theme NAME`, or in the console (`` ` ``):
`video.theme grid`. The change applies immediately.

## Making your own theme

1. Run `lgduel --make-example-theme MyTheme` (or copy `themes/example` from the download) and put the folder in
   `<settings folder>/themes/`. The settings folder is `%APPDATA%\LGDuel` on Windows,
   `~/.config/lgduel` on Linux, `~/Library/Application Support/LGDuel` on macOS.
   (`themes/` next to the game executable is searched too, as is `--theme-dir FOLDER`.)
2. Edit `theme.toml` and the PNG files. Pick the folder's name as the theme.
3. After editing while the game runs, type `reloadtheme` in the console.

Folder layout:

```
<settings folder>/themes/mytheme/
    theme.toml
    floor.png  wall.png  ...        (any names, referenced from theme.toml)
```

## theme.toml

```toml
sky = [0.10, 0.10, 0.16]          # background colour, linear 0..1 (the bottom, with sky_top)
sky_top = [0.0, 0.0, 0.05]        # optional: gradient up to this colour (only seen on maps with open sky)

[edges]                           # optional outlines along the edges of every solid
color = [0.35, 0.65, 1.0]
width = 1.5                       # world units; 0 = off

[defaults]                        # which material an untagged surface uses
floor = "floor"                   # faces pointing up
wall = "wall"                     # side faces
ceiling = "ceiling"               # faces pointing down
ramp = "floor"                    # sloped faces of ramps

[materials.floor]                 # a material: any name you like
texture = "floor.png"             # a PNG in the theme folder, or a built-in name: tiles, concrete, panel (light greys meant to be tinted), or the coloured q_wall, q_floor, q_plate, q_ceiling (use a white `color`)
tint_map_color = true             # multiply by the map's own colour for this surface
color = [1.0, 1.0, 1.0]           # surface colour; leave out to use the map's colour
scale = 128.0                     # world units per texture repeat (a player is 56 tall)
shade = true                      # per-face brightness (tops brighter than sides)

[maps.arena_tiers.materials.floor]   # optional: change a material for one map only
color = [0.6, 0.7, 0.8]
```

Everything is optional. A material you don't define uses the classic look (the map's colour, no texture), so a
theme can be a single `[materials.floor]`. Typos in key names are reported instead of ignored.

**How a surface picks its material:** the map entry's `material = "name"` tag, if the theme defines that name
(per-map override first, then the general one); otherwise the `[defaults]` name for its direction; otherwise
the classic look. Maps tag a few things (see [MAPS.md](MAPS.md)); everything else is untagged.

## Textures

- PNG, any colour type, 8 or 16 bit, at most 2048 × 2048 pixels and 4 MB per file. They are scaled to 256 × 256
  when loaded, so power-of-two squares look best.
- Make them **tile**: the edges should match when repeated.
- Keep them **mid-bright and low-contrast**. The surface colour is multiplied by the texture, and very dark or
  busy walls make enemies hard to see.
- File names are relative to the theme folder (`floor.png`, `tiles/floor.png`); `..`, absolute paths and drive
  letters are refused. At most 32 different textures per theme.
- A problem (missing file, bad PNG, too big) never stops the game: that material is drawn untextured and a line
  appears in the console (`` ` ``) and the terminal, e.g. `theme mytheme: texture floor.png: not found`.

## Visibility

Enemies keep their solid forced colours whatever the theme (Settings → Visuals → Enemy colours), so they stay
easy to see. If you pick colours for the arena, avoid the bright yellow, green and cyan of the default enemy
colours or change those.

## Sharing

Send a theme as one file with `exporttheme NAME` / drag-and-drop import: see [SHARING.md](SHARING.md).

## Licence

Everything in `themes/example` is original and CC0 (public domain). Only use textures you made or may share.

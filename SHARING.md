# Sharing your look

Two ways, depending on what you want to share:

| | What it carries | How big |
|---|---|---|
| **Share code** (`LGP1-…`) | your crosshair, beam, enemy colours and model, skin, HUD look and which theme you use (and the theme itself if it has no image files) | one line of text, paste it anywhere |
| **Theme pack** (`.lgtheme`) | a whole theme folder: `theme.toml` and its PNG textures | one file |

Nothing is ever sent to the game server. Other players don't see your look online; it is how *you* see the game.

## Share codes

**Settings → Share**:
- *Make code* shows your code; *Copy* puts it on the clipboard. Tick *Include controls* to add mouse sensitivity, FOV and volume.
- Paste someone's code into the lower box, press *Preview* to see exactly what it would change, then *Apply*.
- *Presets* applies a built-in look: `classic` (reset), `high-visibility`, `colour-blind-safe`, `minimal`, `quake` (dark stone and steel, fighter enemies, zigzag beam from the bottom middle), `showcase`.

Console (`` ` ``): `exportprofile [controls]`, `importprofile CODE`, `presets`, `preset NAME`. Command line: `lgduel --import-profile CODE`.

**What is never in a code (and ignored if present):** your name, server list, relay address, hosting and practice settings,
window size, fullscreen, vsync and FPS cap. Controls are only applied if you tick *Include controls*.
Codes are checked before use: a damaged code is refused, and a setting with a bad value is skipped while the others apply.

If the code names a theme you don't have, it tells you and leaves your theme alone. Ask for the theme pack.

## Theme packs

- Export: console `exporttheme NAME` writes `themepacks/NAME.lgtheme` next to your settings.
- Import: drag the `.lgtheme` file onto the game window, or console `importtheme FILE [as NAME]`, or `lgduel --import-theme FILE`.
  The theme is installed in your themes folder and selected.

A pack is fully checked before anything is written: only `theme.toml` and `.png` files, sizes limited, every image must be a
valid PNG, the theme must parse. A bad pack installs nothing. An existing theme is never overwritten (import under another name).

## From KovaaK's

Settings → Share → KovaaK's turns your own KovaaK's themes into LGDuel themes (they then travel in codes like any theme) and
copies your hit sound. Sound files are *not* part of a code. See [KOVAAKS.md](KOVAAKS.md).

## Tips
- Share the theme *name* in a code only for built-in themes (`classic`, `grid`); for your own, send the `.lgtheme` too.
- Use `colour-blind-safe` as a starting point and tweak the enemy colours in Settings → Visuals.

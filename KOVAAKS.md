# Using your KovaaK's themes and hit sound

If KovaaK's is installed on this PC, LGDuel can copy your **themes** (wall, floor, ceiling and ramp colours, a matching
texture style, sky colour, optionally your enemy colours) and your **hit and kill sounds**. It only reads your KovaaK's
folder; everything imported is *copied* into LGDuel's own settings folder, so it keeps working if KovaaK's moves or is
uninstalled. Nothing is sent anywhere, and nothing from KovaaK's is part of LGDuel.

## How

Settings → **Share** → **KovaaK's** (scroll down), or the console (`` ` ``):

| Console | Does |
|---|---|
| `kovaaks` | shows what was found |
| `kovaaks themes` | lists your KovaaK's themes |
| `kovaaks import NAME` / `current` / `all` | imports one theme (and uses it), the one selected in KovaaK's, or all of them |
| `kovaaks sounds` | copies your hit sound and kill sound and turns them on |
| `hitsound` / `hitsound FILE` / `hitsound default` | show / pick (a file in `sounds/`) / go back to the built-in hit sound |

From a shell (these print a line and exit; handy for scripts):

```
lgduel --kovaaks-report
lgduel --import-kovaaks-theme current      # or a theme name, or all
lgduel --import-kovaaks-sounds
```

Importing a theme selects it and (with "Use my enemy colours", on by default) sets the three enemy colours from it.
`Import all` adds every theme without selecting one. **Re-import to update:** importing again replaces the copy.

## Where it looks

Steam's library list (`libraryfolders.vdf`) is read to find the game, on Windows and on Linux (Steam, Flatpak Steam, Proton).
If yours is somewhere unusual, put the folder in Settings → Share, or `kovaaks.path "/path/to/steamapps/common/FPSAimTrainer"`,
or start with `--kovaaks-dir PATH`. The folder with `FPSAimTrainer.exe`, the inner `FPSAimTrainer` folder, or a Steam
library folder all work.

## Where things go

| What | Where (next to `settings.toml`) |
|---|---|
| themes | `themes/kovaaks-<name>/theme.toml` (a normal LGDuel theme; your own theme with that name is never overwritten) |
| sounds | `sounds/<file>` (the original file) |

## What is and isn't imported

- **Imported:** colours (exact), texture *style*, texture scale, sky colour, enemy colours, hit sound, kill sound, hit volume (optional).
- **Not imported:** KovaaK's own textures and skyboxes (they live inside the game's `.pak`, which LGDuel never reads), lighting,
  glow, per-weapon and per-scenario sound overrides, sounds that are built into KovaaK's (reported as "can't be imported").
- **Texture style** is the nearest of LGDuel's three built-in ones (tiles, concrete, panel) or a flat colour. A skybox theme becomes
  a gradient in its sky colour.
- **Visibility first:** surfaces keep their per-face shading so walls stay distinguishable, and LGDuel warns when an enemy colour
  is close to the floor or wall colour.
- **Sounds:** Ogg Vorbis and WAV (PCM 8/16/24/32-bit, float). ADPCM WAVs are refused with a message (re-save as 16-bit PCM).
  Only the first 2 seconds are used, and at most four hit sounds overlap.
- Sound file names are not shared in `LGP1-` codes (a name means nothing on another PC).

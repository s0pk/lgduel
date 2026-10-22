# Performance notes

Measured 2026-10-23 on the development PC: AMD Ryzen 7 9700X, NVIDIA GeForce RTX 4060 (driver 32.0.16.1692,
wgpu picks Vulkan), Windows 11. Window 1920x1080, vsync off (present mode Immediate), practice duel with the
bot strafing and the player firing (`--synthetic-input`), 4000 frames.

| Scene | avg FPS | p50 frame | p99 frame | max |
|---|---|---|---|---|
| arena_flat, defaults | 4138 | 0.199 ms | 0.578 ms | 1.44 ms |
| arena_tiers, gun model + input display on | 3933 | 0.211 ms | 0.594 ms | 1.40 ms |
| same, `dist` build (LTO) | 4113 | 0.200 ms | 0.574 ms | 1.55 ms |
| arena_tiers, `grid` theme + `humanoid` enemy (textured world pipeline) | 6675 | 0.141 ms | 0.287 ms | 1.27 ms |

(The later rows ran after other measurements; on this machine run-to-run variation is about ±30%. The textured
world pipeline and the humanoid model cost nothing measurable.)

Target was 500+ FPS at 1080p: met with a large margin (the p99 frame is still ~1700 FPS). The scene is a few
hundred flat-shaded triangles plus HUD quads in one pipeline each, uploaded per frame; there is nothing to
optimise yet. The offscreen check in check.sh (GPU-synchronous per frame) reports ~7000 FPS.

## Input latency path

- Mouse: raw device deltas (`DeviceEvent::MouseMotion`, no OS acceleration), applied to the view on the frame
  they arrive; the rendered view always uses the newest angles, not the last simulated tick's.
- Present: vsync off by default → `Immediate` (no frame queue). Optional FPS cap and vsync in Settings.
- Simulation: fixed 125 Hz; each command carries the angles current at that tick. Rendering interpolates
  own movement between ticks (no extra delay beyond the tick fraction).
- Online: own movement is predicted (bit-exact with the server, so no corrections on a clean link); other
  players are interpolated ~2 ticks behind (adaptive, see net-graph INTERP); hits use anti-lag rewind by
  default, so what you saw when you fired is what counts.

## Binary size and startup

| | release | dist (LTO, stripped) |
|---|---|---|
| lgduel.exe | 15.0 MB | 13.4 MB |
| lgduel-server.exe | 1.36 MB | 1.24 MB |
| lgduel-bot.exe | 0.96 MB | 0.87 MB |

Release packages are built with `--profile dist` (`scripts/package.sh`, release workflow); everyday builds and
check.sh use `release` / `release-dbg` (the `dist` build takes ~2 min). Most of lgduel.exe is wgpu (Vulkan +
DX12 + GL backends, shader compiler) and egui.

Startup: `lgduel --practice` to the first presented frame and exit: ~650 ms (mostly creating the graphics
device). `lgduel-server` start + stop: ~30 ms.

# Map format

Maps are TOML files (`maps/*.toml`) built from simple solids. Units are Quake units (a player is 30 wide and
56 tall; eye height 26; walking speed 320 u/s). +z is up; the floor top is usually z = 0.

```toml
name = "my_map"                  # identifies the map (shown in the browser, used for matching)
default_color = [0.55, 0.55, 0.58]

# Axis-aligned box.
[[brush]]
min = [-1024.0, -1024.0, -32.0]
max = [1024.0, 1024.0, 0.0]
color = [0.35, 0.36, 0.40]       # optional, else default_color

# Wedge inside min..max whose top rises from min.z at the low end to max.z at the side named by `rise`.
[[ramp]]
min = [0.0, -160.0, 0.0]
max = [256.0, 160.0, 96.0]
rise = "+x"                      # "+x" | "-x" | "+y" | "-y"

# `steps` boxes inside min..max climbing toward `rise`; the last reaches max.z.
[[stairs]]
min = [64.0, 560.0, 0.0]
max = [256.0, 752.0, 96.0]
rise = "+x"
steps = 6                        # 1..=64

# Any convex solid: planes [nx, ny, nz, d], inside where nx*x + ny*y + nz*z <= d.
[[convex]]
planes = [[0.0, 0.0, -1.0, 0.0], [1.0, 0.0, 1.0, 300.0], [-1.0, 0.0, 0.0, -200.0],
          [0.0, 1.0, 0.0, 50.0], [0.0, -1.0, 0.0, 50.0]]

# At least two spawn points. `origin` is the player's origin: feet at origin.z - 24.
[[spawn]]
origin = [-768.0, 0.0, 24.0]
yaw = 0.0                        # degrees, 0 = +x, 90 = +y
```

## Materials (visual only)

Any `[[brush]]`, `[[ramp]]`, `[[stairs]]` or `[[convex]]` entry may carry `material = "name"` (for example
`"trim"` or `"cover"`). The game's theme decides what a material looks like ([themes](STATE/HANDOFF_VISUALS.md)
are client-side); collision, spawns and the map's content hash ignore it, so tagging never makes a client and
server disagree about a map. Untagged entries are themed by orientation (floor, wall, ceiling).

## Movement rules worth knowing

- **Steps** up to 18 units are climbed while walking (stairs with 16-unit steps keep full speed).
- **Slopes** up to about 45° (surface normal z ≥ 0.7) are walkable; steeper ones are slid off.
  On a slope the 3D speed stays 320, so horizontal speed is 320·cos(angle).
- Running up a ramp onto a flat top gives a small hop (as in Quake 3).
- Players collide with every solid; there are no water, lava, jump pads or teleporters.

## Checks when loading

Every box needs min < max on each axis; ramps and stairs a valid `rise`; convex solids must enclose a finite
volume; every spawn must have room for a standing player (not inside any solid). Errors name the entry, e.g.
`invalid map: stairs 0: steps must be 1..=64`.

## Built-in maps and custom maps

`arena_flat`, `arena_pillars` and `arena_tiers` are built into the game and the server. A server can also load
a file (`map = "maps/my_map.toml"` or `--map FILE`), but clients currently only play built-in maps: joining a
server whose map (name + geometry) isn't built in fails with a clear message. To share a map today, add it to
`maps/` and to the built-in list (`crates/sim/src/world.rs`, `builtin::ALL`) and rebuild. Map downloads are
planned.

Try a map offline: `lgduel.exe --practice --map arena_tiers`.

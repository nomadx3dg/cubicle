# cubicle

Interactive 3D renderer for your terminal. It draws lit, spinning 3D models as ASCII art, coloured ASCII, or half-block colour, and turns PNGs into ASCII. Works with keyboard, mouse and touch (Termux). Start it with no arguments for a main menu with render settings and a file browser.

The default build is a pure CPU rasteriser with two dependencies, `crossterm` (terminal I/O) and `png` (decoding), and runs anywhere. An optional **GPU backend** (`--features gpu`) renders on the GPU via `wgpu`, or on the CPU and GPU together.

## Run

```sh
cargo run --release                         # the main menu (saved settings)
cargo run --release -- --cube               # straight to the built-in cube
cargo run --release -- models/torus.obj     # any Wavefront .obj
cargo run --release -- -m blocks -p amber models/sphere.obj
cargo run --release -- examples/gradient.png
cargo run --release -- --texture examples/checker.png -m blocks models/sphere.obj
```

### Termux (Android)

```sh
pkg install rust git
git clone https://github.com/nomadx3dg/cubicle
cd cubicle
cargo run --release -- -m blocks
```

## Main menu

Running `cubicle` with no arguments opens the menu (`--menu` opens it with your command-line flags as the starting settings):

| Item | What it does |
| --- | --- |
| Spin the cube | The built-in demo cube |
| Browse models (.obj)… | File browser: pick a model and view it. Remembers the last folder |
| View an image (.png)… | The same browser, filtered to PNGs |
| Render settings… | Backend, render mode, palette, colours, shading, faces, auto-spin, texture |
| Quit | |

In a viewer, `q` / Esc returns to the menu. A model that fails to load shows its error in the menu instead of closing the program.

**Render settings** are saved after every change to `~/.config/cubicle/settings.conf` (or `$XDG_CONFIG_HOME/cubicle/`, or `$CUBICLE_CONFIG`), a plain `key=value` file you can edit by hand.

| Setting | Options |
| --- | --- |
| Backend | `cpu`, `gpu`, `cpu+gpu` (see below) |
| Render mode | `ascii`, `color`, `blocks` |
| Palette | `natural`, `green`, `amber`, `ice` |
| Colours | `auto`, `truecolor`, `256`, `16` |
| Shading | `smooth`, `flat` |
| Faces | `culled`, `two-sided` |
| Auto-spin | on / off |
| Texture | none, or a PNG chosen with the browser (`x` removes it) |

**Menu controls:** ↑/↓ (or `j`/`k`) move, Enter or → selects, ← / → change a setting, `1`-`5` jump to a main-menu item, Esc / `q` go back. **Touch:** tap an item to select it; in settings the left third of a row steps back and the rest steps forward. In the file browser, the first tap selects and tapping the selected entry opens it (so a stray tap never launches a file); tap the title bar to go back; swipe to scroll. In the browser: Enter opens, Backspace goes up, `h` shows hidden files.

## Render backends

| Backend | Who draws the frame |
| --- | --- |
| `cpu` | The software rasteriser (default; works everywhere). |
| `gpu` | A `wgpu` render pipeline on the GPU; the finished frame is read back and encoded to text. |
| `cpu+gpu` | Both at once: the **GPU draws the top half of the frame while the CPU draws the bottom half**, in parallel, and the halves are joined. |

Choose it in the menu, with `--backend cpu|gpu|both`, or live in a viewer with `g` or the backend button in the bottom bar (`cpu*` means the GPU was requested but could not be used).

The GPU backend is optional and off by default so that the default build stays small and works on devices with no GPU stack (Termux):

```sh
cargo run --release --features gpu -- --backend gpu models/sphere.obj
```

It needs a Vulkan, Metal or DirectX 12 driver. If the build has no `gpu` feature, or no adapter is found, or the GPU fails mid-run, cubicle falls back to the CPU and says why (in the settings page, the `cpu*` label, and on stderr for `--once`). CPU and GPU frames agree to within rounding: tests compare them pixel by pixel.

## Options

| Option | Meaning |
| --- | --- |
| `FILE` | `.obj` model or `.png` image (none: the main menu) |
| `-m, --mode ascii\|color\|blocks` | Plain ASCII (default), coloured ASCII, or half-block colour |
| `-p, --palette natural\|green\|amber\|ice` | Colour palette for `color` / `blocks` |
| `--colors auto\|truecolor\|256\|16` | Colour depth to emit. `auto` (default) reads `COLORTERM` / `TERM` |
| `--texture FILE.png` | Map a PNG onto the model using its `vt` coordinates (the built-in cube and `models/sphere.obj` have them) |
| `--backend cpu\|gpu\|both` | Who draws models (see *Render backends*) |
| `--menu` / `--cube` | Open the main menu / skip it and show the cube |
| `--flat` | Start with flat shading instead of smooth |
| `--two-sided` | Draw back faces too (open meshes, planes) |
| `--crease <deg>` | Smooth-shading crease angle, default 60 |
| `--no-auto` | Start with auto-spin off |
| `--invert` | Invert brightness for light terminals (images) |
| `--once` | Print one frame to stdout and exit, no TTY needed |
| `--size WxH` | Frame size for `--once` (default: terminal size, else 80x24) |
| `--rot X,Y` | Starting rotation in degrees (also used by `--once`) |

`--once` makes cubicle usable in scripts and tests: `cubicle --once --size 60x20 -m blocks model.obj`.

## Controls

| Input | Action |
| --- | --- |
| Tap the left / right / top / bottom of the screen | Turn the model that way |
| Tap the center | Auto-spin on / off |
| Swipe up / down | Tilt the model |
| Bottom bar `[ - ]` `[ auto ]` `[ mode ]` `[ backend ]` `[ + ]` | Zoom out, auto-spin, cycle render mode, cycle backend, zoom in |
| Arrow keys | Turn the model |
| `+` / `-` | Zoom |
| Space | Auto-spin on / off |
| `m` | Cycle mode: ascii, color, blocks |
| `c` | Cycle palette |
| `s` | Smooth / flat shading |
| `b` | Two-sided / back-face culling |
| `g` | Cycle backend: cpu, gpu, cpu+gpu |
| `t` | Texture on / off (when `--texture` is given) |
| `r` | Reset view |
| Mouse drag | Rotate (desktop terminals) |
| `q` / Esc | Quit |

Image viewer: `m` mode, `c` palette, `i` invert, `q` quit; the bottom bar `[ mode ]` `[ color ]` `[ invert ]` works by touch.

Termux reports taps and up/down swipes to programs but not sideways finger movement, so touch turning is tap-based.

## Models

OBJ support: `v` (with optional `r g b` vertex colours), `vn`, and `f` in every index form (`v`, `v/vt`, `v//vn`, `v/vt/vn`) with negative indices. Polygons are fan-triangulated, so they must be convex. Models are centred and scaled to fit automatically; inside-out (clockwise) winding is detected and repaired; missing normals are generated. `vt` texture coordinates are kept (see `--texture`). Materials and groups are ignored. Models are viewed right-handed from +z: `+x` is right, `+y` is up and `+z` points at you, so a model that faces +z shows its front at start.

Samples in `models/`: `sphere.obj` (normals and texture coordinates), `torus.obj` (quads, no normals), `pyramid.obj` (vertex colours, negative indices, clockwise winding).

## How it works

Triangles are projected with perspective, rasterised with barycentric coordinates into a depth buffer (1/z), lit per pixel with a point light, and mapped to characters from `" .,:;-=+*#%@"`. Smooth shading interpolates per-corner normals (averaged across faces within the crease angle, so cube edges stay sharp and spheres go round). Half-block mode draws two square pixels per cell with `▀`/`▄` and foreground and background colours (24-bit, 256-colour or 16-colour, see `--colors`).

For contributors and AI agents, see [`handout.md`](handout.md).

## Development

```sh
cargo test                    # unit + end-to-end + pty tests
cargo test --features gpu     # also compares the GPU renderer to the CPU one (skips without an adapter)
cargo clippy --all-targets
cargo fmt
```

## Roadmap

- [x] `.obj` loading
- [x] Smooth per-vertex shading
- [x] Half-block characters and color
- [x] PNG to ASCII
- [x] Texture-mapping PNGs onto models (`vt` coordinates)
- [ ] `.mtl` materials
- [x] 256- and 16-colour fallback (`--colors`)
- [x] Main menu, render settings, `.obj` browser
- [x] GPU and CPU+GPU render backends (optional `gpu` feature)

## License

MIT

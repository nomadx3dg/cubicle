# cubicle

Interactive 3D renderer for your terminal. A CPU rasteriser that draws lit, spinning 3D models as ASCII art, coloured ASCII, or half-block colour. It also turns PNGs into ASCII. Works with keyboard, mouse and touch (Termux).

No GPU, no graphics libraries. Dependencies: `crossterm` (terminal I/O) and `png` (decoding).

## Run

```sh
cargo run --release                         # the built-in cube
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

## Options

| Option | Meaning |
| --- | --- |
| `FILE` | `.obj` model or `.png` image (none: built-in cube) |
| `-m, --mode ascii\|color\|blocks` | Plain ASCII (default), coloured ASCII, or half-block colour |
| `-p, --palette natural\|green\|amber\|ice` | Colour palette for `color` / `blocks` |
| `--colors auto\|truecolor\|256\|16` | Colour depth to emit. `auto` (default) reads `COLORTERM` / `TERM` |
| `--texture FILE.png` | Map a PNG onto the model using its `vt` coordinates (the built-in cube and `models/sphere.obj` have them) |
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
| Bottom bar `[ - ]` `[ auto ]` `[ mode ]` `[ + ]` | Zoom out, auto-spin, cycle render mode, zoom in |
| Arrow keys | Turn the model |
| `+` / `-` | Zoom |
| Space | Auto-spin on / off |
| `m` | Cycle mode: ascii, color, blocks |
| `c` | Cycle palette |
| `s` | Smooth / flat shading |
| `b` | Two-sided / back-face culling |
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
cargo test          # unit + end-to-end tests
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

## License

MIT

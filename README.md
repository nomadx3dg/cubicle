# cubicle

Interactive 3D renderer for your terminal. A CPU rasteriser that draws a lit, spinning cube as ASCII art. Works with keyboard, mouse and touch (Termux).

## Run

```sh
cargo run --release
```

### Termux (Android)

```sh
pkg install rust git
git clone https://github.com/nomadx3dg/cubicle
cd cubicle
cargo run --release
```

## Controls

| Input | Action |
| --- | --- |
| Tap left/right/top/bottom of the screen | Spin that way |
| Tap the center | Pause / play |
| Drag | Rotate |
| Swipe up / down | Zoom in / out |
| Bottom bar `[ - ]` `[ pause ]` `[ + ]` | Zoom out, pause, zoom in |
| Arrow keys | Rotate |
| `+` / `-` | Zoom |
| Space | Pause |
| `q` / Esc | Quit |

## How it works

Triangles are projected with perspective, rasterised with barycentric coordinates into a depth buffer (1/z), shaded per face with a directional light, and mapped to characters from `" .:-=+*#%@"`. No GPU, no graphics libraries. The only dependency is `crossterm` for terminal I/O.

## Roadmap

- [ ] `.obj` loading
- [ ] Smooth per-vertex shading
- [ ] Half-block characters and color
- [ ] PNG to ASCII

## License

MIT

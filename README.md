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
| Tap the left / right / top / bottom of the screen | Turn the cube that way |
| Tap the center | Auto-spin on / off |
| Swipe up / down | Tilt the cube |
| Bottom bar `[ - ]` `[ auto ]` `[ + ]` | Zoom out, auto-spin, zoom in |
| Arrow keys | Turn the cube |
| `+` / `-` | Zoom |
| Space | Auto-spin on / off |
| Mouse drag | Rotate (desktop terminals) |
| `q` / Esc | Quit |

Termux reports taps and up/down swipes to programs but not sideways finger movement, so touch turning is tap-based.

## How it works

Triangles are projected with perspective, rasterised with barycentric coordinates into a depth buffer (1/z), lit per pixel with a point light, and mapped to characters from `" .:-=+*#%@"`. No GPU, no graphics libraries. The only dependency is `crossterm` for terminal I/O.

## Roadmap

- [ ] `.obj` loading
- [ ] Smooth per-vertex shading
- [ ] Half-block characters and color
- [ ] PNG to ASCII

## License

MIT

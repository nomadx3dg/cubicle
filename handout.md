# cubicle — handout for AI developers

This document explains what changed in v0.2.0, how the code is organised, the decisions behind it, and where to go next. Read it before editing; it is meant to save you re-deriving things.

## 1. Where the project stands

`cubicle` is a terminal 3D viewer written in Rust. It started (v0.1.0) as one 430-line `main.rs` that drew a lit spinning cube with a keyboard/mouse/Termux-touch UI. v0.2.0 completes the original README roadmap:

| Roadmap item | Status | Where |
| --- | --- | --- |
| `.obj` loading | done | `src/obj.rs`, `src/mesh.rs` |
| Smooth per-vertex shading | done | `src/mesh.rs` (`smooth_normals`), `src/raster.rs` |
| Half-block characters and color | done | `src/display.rs` (`Mode`, `Palette`, `Canvas::encode`) |
| PNG to ASCII | done (as an image viewer, see §6) | `src/image.rs`, `src/app.rs` |

Also added: a CLI (`--once`, `--size`, `--rot`, ...), a library/binary split, 53 tests, sample assets, CI config, and a refreshed README.

Toolchain: developed and tested with **Rust 1.75** (`rust-version = "1.75"`), `crossterm 0.27`, `png 0.17`. `Cargo.lock` is committed.

## 2. Module map and data flow

```
 cli.rs ──► app.rs ──┬─► obj.rs ─► mesh.rs ─┐
 (Config)   (loops,  │                      ├─► raster.rs ─► Canvas ─┐
            input)   └─► image.rs ──────────┘    (3D → pixels)       ├─► display.rs ─► terminal rows
                         (PNG → pixels, directly into Canvas) ───────┘   (Canvas::encode, hud)
```

| File | Responsibility |
| --- | --- |
| `src/vec3.rs` | `V3` with method-style maths (`a.add(b)`). Clippy's `should_implement_trait` is deliberately allowed. |
| `src/mesh.rs` | `Mesh`/`Triangle`/`RawTri`. `Mesh::from_raw` centres + scales to `TARGET_RADIUS` (√3, the cube's circumradius), repairs winding, generates smooth normals. `Mesh::cube()` has per-face colours. |
| `src/obj.rs` | OBJ parser (`parse`) and file loader (`load`). Errors carry line numbers. |
| `src/raster.rs` | `render(mesh, ax, ay, cam, RenderOpts, &mut Frame)`. Projection, backface culling / two-sided, barycentric fill, 1/z depth buffer, per-pixel point light, optional interpolated normals. |
| `src/display.rs` | `Canvas` (pixel grid), `Mode`, `Palette`, `Canvas::encode` → `Vec<String>` rows, and the HUD helpers `hud` / `hud_hit`. |
| `src/image.rs` | `Image::decode/load` (any PNG colour type/bit depth → 8-bit RGB over black) and `Image::to_canvas` (aspect-correct fit + box filter). |
| `src/cli.rs` | Hand-rolled argument parser → `Config`. `USAGE` text lives here. |
| `src/app.rs` | `TermGuard` (raw mode/alt screen, always restored), `Screen` (row-diff drawing in synchronized-update blocks), `ModelView` / `ImageView` (state + input handlers), the two loops, and `render_once`. |
| `src/main.rs` | Thin: parse args, call `app::run`, map errors to exit codes (1 = runtime, 2 = usage). |
| `tests/cli.rs` | End-to-end tests running the real binary with `--once` on `models/` and `examples/`. |

### The key abstraction: `Canvas`

Everything visual funnels through `display::Canvas { w, h, lum, base, lit }`:

- `lum` is brightness 0..1 and drives glyph choice; **negative means empty**.
- `base` is a per-pixel RGB colour (0..1).
- `lit == true` (3D): drawn colour = `base × tone(lum)`. `lit == false` (images): `base` is the final colour and `lum` is just luminance for the glyph.
- `Mode::Blocks` packs two canvas rows into one terminal row, so the canvas height is `rows × 2` there. `Mode::pixels_per_row()` gives the factor; both `render` and `Image::to_canvas` take it. An odd canvas height drops its last row.

Because `base` is per-pixel rather than per-triangle, texture mapping needs no change to display code (see §7).

## 3. Behaviour you should know about

**Geometry and shading**
- Camera sits at `(0, 0, -cam)` looking toward +z; the model rotates (`rot_y` then `rot_x`), the camera does not. The light is a fixed point light in that same space, near the camera, up and to the right. Lighting formula is unchanged from v0.1.0: `0.08 + 1.9·diffuse·attenuation`, clamped to 1.
- Terminal cells are ~2× taller than wide. In ASCII mode the horizontal projection scale is doubled; in block mode pixels are square, so `render` works in "cell units" and converts (`focal_cells`, `focal`, `x_scale`). A test (`blocks_mode_keeps_the_same_physical_size`) guards this.
- Smooth normals: positions are welded by quantising to 1e-4 (after normalisation), then each corner averages the area-weighted normals of faces at that position that are within the crease angle (default 60°) of its own face. Consequences: the cube stays crisp, curved meshes go smooth, and `--crease` tunes it. File-supplied `vn` normals take precedence when all three corners have one.
- `--flat` / `s` ignores corner normals and uses the geometric face normal (the v0.1.0 look).
- Winding: after centring, a negative signed volume flips all triangles. This is a heuristic that is reliable for closed meshes only. For open meshes use `--two-sided` / `b`, which flips the normal of any back-facing triangle and lights it from its own side instead of culling.
- Triangles with any vertex nearer than z = 0.1 are dropped whole (no near-plane clipping). `CAM_MIN` (2.6) keeps normal models clear of this.

**Colour**
- Truecolor only (`ESC[38;2;r;g;bm` / `48;2`). `RowWriter` emits a colour escape only when it changes and resets at row end. Mono ASCII mode emits no escape codes at all.
- `Palette::apply` tints by luma (Natural = untouched). Shadows are lifted in block mode (`0.22 + 0.78·lum^0.7`) because the original lighting is harsh and unlit faces went nearly black.
- The default render mode is still plain ASCII to preserve the v0.1.0 look and maximum terminal compatibility.

**Input**
- The bottom terminal row is the HUD, so the 3D area is `rows - 1` tall.
- Model HUD has four equal touch zones: `[ - ] [ auto ] [ mode ] [ + ]` (v0.1.0 had three). Image HUD has three: mode, color, invert. `hud_hit` and `hud` share one `zones()` function so drawing and hit-testing cannot drift apart.
- Input handlers are plain methods on `ModelView`/`ImageView` (`on_key`, `on_mouse`, `on_tap`) so they are testable with no terminal.
- Termux sends taps as left clicks and vertical swipes as scroll events, but no horizontal movement; that is why turning is tap-based. Unchanged from v0.1.0.

**Image viewer**
- Redraws only on input/resize (event poll with a 250 ms timeout), unlike the model loop's fixed ~30 fps.
- Alpha is composited onto black. Downscaling is a box filter in sRGB space (not linear); upscaling is nearest-neighbour via the same box maths.

**Models**
- Supported OBJ: `v` (+ optional `r g b`), `vn`, `f` in all index forms incl. negative indices. Ignored: `vt`, `o`, `g`, `s`, `usemtl`, `mtllib`. Polygons are fan-triangulated, which is wrong for concave polygons.
- Per-triangle colour = average of its vertex colours if all three have one, else light grey (0.85).

## 4. Testing

```sh
cargo test                       # 45 unit tests + 8 end-to-end tests
cargo clippy --all-targets       # clean as of 1.75
cargo fmt                        # rustfmt.toml: max_width = 110
```

- `--once --size WxH [--rot X,Y] [-m mode]` renders one deterministic frame to stdout with no TTY. Use it for regression checks and for eyeballing changes.
- For a visual check of colour modes, pipe `--once -m blocks` through a small ANSI→PNG script (parse `38;2`/`48;2`, draw `▀`/`▄` as two pixels). One was used during development; it is not in the repo.
- The interactive loops were smoke-tested through a pseudo-terminal (keys, SGR mouse taps, quit paths, terminal restored on exit) but there is no automated pty test in the repo. A good addition.
- Sample assets (`models/*.obj`, `examples/gradient.png`) were generated by throwaway Python scripts; they are plain files and not regenerated by any build step.

## 5. Conventions

- Keep dependencies minimal. Adding a crate needs a reason; the CLI parser is hand-rolled on purpose.
- Errors at boundaries (files, args) are `Result<_, String>` with a human-readable message; `main` prints `cubicle: <msg>` and sets the exit code. Do not `unwrap` on user input.
- Hot-loop code (`Frame::fill`) allocates nothing per pixel. Normals and meshes are precomputed at load, not per frame.
- Anything stateful in the UI goes on `ModelView`/`ImageView` with a unit test, not inline in the loop.
- Keep `Mode::Ascii` free of escape sequences (a test asserts it).

## 6. Assumptions and open questions

These were judgement calls made without being able to ask. Confirm or overrule them:

1. **"PNG to ASCII" means an image viewer** (`cubicle pic.png`), not texturing a model. Texturing is listed as the next roadmap item. If the author meant texture mapping, §7 describes the approach.
2. **Default mode stays plain ASCII**, and colour/blocks are opt-in. Switching the default to blocks would look better on modern terminals but changes v0.1.0 behaviour.
3. **Truecolor is assumed** for colour modes (fine for Termux and modern desktop terminals). There is no 256-colour or 16-colour fallback, and no auto-detection via `COLORTERM`.
4. **Version bumped to 0.2.0** and a `[lib]` target added. Revert the bump if the author prefers to version differently.
5. **Not verified on real Termux hardware or Windows.** Verified on Linux with a pty. CI config for Linux/macOS/Windows is included but has never run.
6. **The README's old character ramp text was stale** (`" .:-=+*#%@"`); the code's actual ramp is `" .,:;-=+*#%@"`. The README now matches the code.
7. **Push status:** the work was committed locally; whether it reached GitHub is covered in the delivery notes outside this file. Note that pushing `.github/workflows/ci.yml` requires a token with the `workflow` scope.

## 7. Suggested next steps

In rough priority order.

1. **Texture mapping.** Parse `vt` into `RawTri`/`Triangle` as `uv: [[f32;2];3]`; carry it in `Prim`; in `Frame::fill` interpolate UVs with the *perspective-correct* weights already computed (`a0, a1, a2`, divide by their sum); sample an `Image` (add a `sample(u, v) -> [f32;3]` method with nearest/bilinear lookup) and write the result to `canvas.base[i]` instead of the triangle colour. `lit == true` then multiplies it by lighting, so no display changes are needed. Add `--texture FILE.png` and keep untextured behaviour as the default.
2. **256-colour / 16-colour fallback** and `COLORTERM` detection in `display.rs`. The natural seam is `RowWriter` (it already owns all colour escapes) plus a new `ColorDepth` setting next to `Mode`.
3. **`.mtl` support** (`Kd` diffuse colours via `mtllib`/`usemtl`) feeding the existing per-triangle `color`.
4. **Near-plane clipping** so that zooming inside a model clips instead of dropping triangles.
5. **Concave polygon triangulation** (ear clipping) in `obj.rs`.
6. **Performance** for very large meshes: precompute rotation once per frame per vertex rather than per triangle corner (each shared vertex is rotated up to ~6 times now), and add a bounding-box reject. Measured: an 80k-triangle sphere loads and renders one 120×40 block frame in ~80 ms total including process start, so this is not urgent.
7. **Automated pty smoke test** (the Python script approach used during development) and a snapshot test of `--once` output for the cube.
8. **Zoom/pan in the image viewer** and an image `--contrast` option (auto-levels) for dark photos.

## 8. Gotchas

- The `lit` flag on `Canvas` changes what `base` means; do not multiply colours by `lum` yourself outside `Canvas::block_rgb`/`glyph_rgb`.
- Block mode needs an even canvas height; callers allocate `rows × pixels_per_row`.
- `Screen::draw` diffs rows by string equality. Anything that changes escape sequences between otherwise identical frames will defeat the diff (and cost bandwidth over slow links such as SSH).
- Terminal state is restored by `TermGuard::drop`. Keep all code between guard creation and exit panic-safe, or the user's terminal is left in raw mode.
- `\x1b[?2026h/l` (synchronized update) and `\x1b[?7l` (no line wrap) are written raw; terminals that do not support 2026 simply ignore it.
- `Mesh::cube()` goes through `from_raw`, so any change to normalisation (centre, radius, winding) also affects the cube. The test `cube_has_twelve_outward_triangles` will catch inversions.

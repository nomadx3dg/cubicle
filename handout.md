# cubicle — handout for AI developers

This document explains what changed in v0.2.0 and v0.3.0 (menu, browser, GPU), how the code is organised, the decisions behind it, and where to go next. Read it before editing; it is meant to save you re-deriving things.

## 1. Where the project stands

`cubicle` is a terminal 3D viewer written in Rust. It started (v0.1.0) as one 430-line `main.rs` that drew a lit spinning cube with a keyboard/mouse/Termux-touch UI. v0.2.0 completes the original README roadmap:

| Roadmap item | Status | Where |
| --- | --- | --- |
| `.obj` loading | done | `src/obj.rs`, `src/mesh.rs` |
| Smooth per-vertex shading | done | `src/mesh.rs` (`smooth_normals`), `src/raster.rs` |
| Half-block characters and color | done | `src/display.rs` (`Mode`, `Palette`, `Canvas::encode`) |
| PNG to ASCII | done (as an image viewer, see §6) | `src/image.rs`, `src/app.rs` |
| Texture mapping (`--texture`) | done | `src/obj.rs` (`vt`), `src/raster.rs` (`render_textured`), `Image::sample` |
| 256/16-colour fallback, pty tests | done by a previous session | `display.rs`, `tests/pty.rs` |
| Main menu, render settings, `.obj`/`.png` browser | done | `src/menu.rs`, `src/browser.rs`, `src/settings.rs`, `app::run_menu` |
| CPU / GPU / CPU+GPU backends | done (GPU is an optional cargo feature) | `src/engine.rs`, `src/gpu.rs`, `src/gpu.wgsl` |

Also added: a CLI (`--once`, `--size`, `--rot`, ...), a library/binary split, sample assets, CI config, and a refreshed README. See §9 for the menu and GPU work, which is the newest part.

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
- Camera sits at `(0, 0, +cam)` looking toward -z with x right and y up (right-handed, so models and textures are **not** mirrored: a counter-clockwise triangle faces the viewer). The model rotates, the camera does not. Rotation angles are applied negated (`rot_y(-ay).rot_x(-ax)`) so that every control, `--rot` value and the auto-spin look exactly as they did in v0.1.0 (which viewed the world from -z, i.e. mirrored; the symmetric cube hid this). `tests/` include `view_is_not_mirrored` to guard it. Do not "simplify" the negations away. The light is a fixed point light in that same space, near the camera, up and to the right. Lighting formula is unchanged from v0.1.0: `0.08 + 1.9·diffuse·attenuation`, clamped to 1.
- Terminal cells are ~2× taller than wide. In ASCII mode the horizontal projection scale is doubled; in block mode pixels are square, so `render` works in "cell units" and converts (`focal_cells`, `focal`, `x_scale`). A test (`blocks_mode_keeps_the_same_physical_size`) guards this.
- Smooth normals: positions are welded by quantising to 1e-4 (after normalisation), then each corner averages the area-weighted normals of faces at that position that are within the crease angle (default 60°) of its own face. Consequences: the cube stays crisp, curved meshes go smooth, and `--crease` tunes it. File-supplied `vn` normals take precedence when all three corners have one.
- `--flat` / `s` ignores corner normals and uses the geometric face normal (the v0.1.0 look).
- Winding: after centring, a negative signed volume flips all triangles. This is a heuristic that is reliable for closed meshes only. For open meshes use `--two-sided` / `b`, which flips the normal of any back-facing triangle and lights it from its own side instead of culling.
- Triangles with any vertex nearer than z = 0.1 are dropped whole (no near-plane clipping). `CAM_MIN` (2.6) keeps normal models clear of this.

**Colour**
- Three colour depths (`display::ColorDepth`): truecolor (`ESC[38;2;r;g;bm` / `48;2`), xterm 256 (`38;5;n`), and 16 basic colours (`30-37`/`90-97`, `40-47`/`100-107`). `--colors auto` (the default) resolves via `ColorDepth::detect()`: `COLORTERM=truecolor|24bit`, `WT_SESSION`, known `TERM_PROGRAM`s and `TERM` names (kitty, alacritty, ...) give truecolor; `TERM` containing `256color` gives 256; any other `TERM` gives 16; no `TERM` at all (pipes, CI) keeps truecolor. `Config::default()` is always truecolor so unit tests don't depend on the environment; only `parse_args` calls `detect()`. `RowWriter` quantises through `Paint`, so it emits a colour escape only when the *quantised* colour changes (low depths shrink the stream a lot) and resets at row end. `rgb_to_16` is hue-based rather than nearest-colour, because nearest-colour sends every dim tint to grey. Mono ASCII mode emits no escape codes at all.
- `Palette::apply` tints by luma (Natural = untouched). Shadows are lifted in block mode (`0.22 + 0.78·lum^0.7`) because the original lighting is harsh and unlit faces went nearly black.
- The default render mode is still plain ASCII to preserve the v0.1.0 look and maximum terminal compatibility.

**Textures**
- `Triangle.uv` / `RawTri.uv` hold per-corner `[u, v]` (`v` = 0 is the bottom edge, OBJ convention). `obj.rs` keeps `vt` when a face supplies all three corners. The built-in cube has UVs on every face, oriented so an image reads correctly on all six (test: `cube_texture_is_not_mirrored_on_any_face`).
- `raster::render_textured(.., tex: Option<&Image>, ..)` interpolates UVs with the perspective-correct weights `a0..a2` and writes `Image::sample(u, v)` (bilinear, wrapping) to `canvas.base`; `lit` shading and palettes then apply as usual. `render` is the untextured wrapper. Triangles without UVs keep their own colour (so the torus is unchanged by `--texture`).
- `ModelView` owns `texture: Option<Image>` and `tex_on` (key `t`, a no-op without a texture). `app::build_view` loads `--texture`. `--texture` with a PNG input is a usage error (exit 2).
- Sampling uses the PNG's sRGB bytes as-is (no gamma conversion), consistent with the image viewer. There is no mip-mapping, so minified textures shimmer when spinning; with ~100 columns this is hard to see.
- `examples/checker.png` has an asymmetric red "F" specifically so mirroring/rotation mistakes are obvious.

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
cargo test                       # 91 unit + 16 end-to-end + 12 pty tests
cargo test --features gpu        # + 5 GPU parity tests (they skip themselves when no adapter exists)
cargo clippy --all-targets       # clean as of 1.75, also with --features gpu
cargo fmt                        # rustfmt.toml: max_width = 110
```

- `--once --size WxH [--rot X,Y] [-m mode]` renders one deterministic frame to stdout with no TTY. Use it for regression checks and for eyeballing changes.
- For a visual check of colour modes, pipe `--once -m blocks` through a small ANSI→PNG script (parse `38;2`/`48;2`, draw `▀`/`▄` as two pixels). One was used during development; it is not in the repo.
- `tests/pty.rs` (unix only) runs the real binary in a pseudo-terminal via `libc::openpty` (a unix-only dev-dependency; libc is already in the tree through crossterm). It sends keys and SGR mouse taps, waits for the expected HUD labels / escape sequences, and checks that quitting exits 0, leaves the alternate screen, and restores canonical mode (`ICANON`) on the tty. Add new interactive behaviour there with `Session::start(&[...])`, `wait_for`, `send`. Ctrl-C is intentionally not covered: quitting is `q` / Esc only.
- Sample assets (`models/*.obj`, `examples/gradient.png`) were generated by throwaway Python scripts; they are plain files and not regenerated by any build step.

## 5. Conventions

- Keep dependencies minimal. Adding a crate needs a reason; the CLI parser is hand-rolled on purpose.
- Errors at boundaries (files, args) are `Result<_, String>` with a human-readable message; `main` prints `cubicle: <msg>` and sets the exit code. Do not `unwrap` on user input.
- Hot-loop code (`Frame::fill`) allocates nothing per pixel. Normals and meshes are precomputed at load, not per frame.
- Anything stateful in the UI goes on `ModelView`/`ImageView` with a unit test, not inline in the loop.
- Keep `Mode::Ascii` free of escape sequences (a test asserts it).

## 6. Assumptions and open questions

These were judgement calls made without being able to ask. Confirm or overrule them:

1. **"PNG to ASCII" means an image viewer** (`cubicle pic.png`). Texture mapping was added as well (`--texture`), so both readings of the roadmap item are covered.
2. **Default mode stays plain ASCII**, and colour/blocks are opt-in. Switching the default to blocks would look better on modern terminals but changes v0.1.0 behaviour.
3. **Colour depth is auto-detected** from the environment (see §3). A truecolor terminal that sets neither `COLORTERM` nor a recognisable `TERM` (some tmux/ssh setups) will be treated as 256-colour; `--colors truecolor` overrides. The detection rules are a best guess and have not been tried on real Termux, macOS Terminal or Windows Terminal.
4. **Versions bumped** (0.2.0, then 0.3.0 for the menu and GPU work) and a `[lib]` target added. Revert if the author prefers to version differently.
5. **Not verified on real Termux hardware or Windows.** Verified on Linux with a pty. CI config for Linux/macOS/Windows is included but has never run.
6. **The README's old character ramp text was stale** (`" .:-=+*#%@"`); the code's actual ramp is `" .,:;-=+*#%@"`. The README now matches the code.
7. **Pushing:** agent sessions in this project have had no GitHub credentials, so the maintainer pushes (via a git bundle). Pushing `.github/workflows/ci.yml` requires a token with the `workflow` scope.
8. **Handedness fix (behaviour change):** the original renderer viewed the world mirrored (camera at -z, x to the right). Asymmetric models and textures came out left-right reversed. The camera now sits at +z (right-handed) and angles are negated, so controls and the cube's appearance are byte-identical to before (verified by diffing `--once` output across rotations) while asymmetric content is now correct. If the author *wanted* the old mirrored convention, revert the three lines in `raster::render_textured` (eye, light, `z = cam - v.z`, rotation signs) and flip `u` on the cube.
9. **Menu / backend decisions made without asking** (confirm or overrule): bare `cubicle` now opens the menu (use `--cube` for the old behaviour); "both" means a **split frame** (GPU top half, CPU bottom half) rather than, say, rendering twice and comparing or alternating frames; the GPU backend is an **optional cargo feature** rather than always compiled (a GPU stack would otherwise be mandatory for Termux users); the `.obj` browser is also reused for PNG images and textures; settings persist automatically on every change.

## 7. Suggested next steps

In rough priority order.

1. ~~Texture mapping~~ — done (§3 *Textures*). Possible follow-ups: per-material textures via `.mtl` `map_Kd`, a `--texture-filter nearest` option, mip-mapping.
2. ~~256-colour / 16-colour fallback and `COLORTERM` detection~~ — done (`ColorDepth`, `--colors`).
3. **`.mtl` support** (`Kd` diffuse colours and `map_Kd` textures via `mtllib`/`usemtl`) feeding the existing per-triangle `color` / texture. `map_Kd` would need a per-triangle texture index rather than the single global `--texture`.
4. **Near-plane clipping** so that zooming inside a model clips instead of dropping triangles.
5. **Concave polygon triangulation** (ear clipping) in `obj.rs`.
6. **Performance** for very large meshes: precompute rotation once per frame per vertex rather than per triangle corner (each shared vertex is rotated up to ~6 times now), and add a bounding-box reject. Measured: an 80k-triangle sphere loads and renders one 120×40 block frame in ~80 ms total including process start, so this is not urgent.
7. ~~Automated pty smoke test~~ — done (`tests/pty.rs`). Still open: a snapshot test of `--once` output for the cube.
8. **Zoom/pan in the image viewer** and an image `--contrast` option (auto-levels) for dark photos.

## 8. Gotchas

- The `lit` flag on `Canvas` changes what `base` means; do not multiply colours by `lum` yourself outside `Canvas::block_rgb`/`glyph_rgb`.
- Block mode needs an even canvas height; callers allocate `rows × pixels_per_row`.
- `Screen::draw` diffs rows by string equality. Anything that changes escape sequences between otherwise identical frames will defeat the diff (and cost bandwidth over slow links such as SSH).
- Terminal state is restored by `TermGuard::drop`. Keep all code between guard creation and exit panic-safe, or the user's terminal is left in raw mode.
- `\x1b[?2026h/l` (synchronized update) and `\x1b[?7l` (no line wrap) are written raw; terminals that do not support 2026 simply ignore it.
- `Mesh::cube()` goes through `from_raw`, so any change to normalisation (centre, radius, winding) also affects the cube. The test `cube_has_twelve_outward_triangles` will catch inversions.

## 9. Main menu, settings, file browser and GPU backend

### Menu architecture
- `src/menu.rs` is a state machine (`Menu::on_key`, `on_mouse`, `draw`) returning an `Action` (`None`, `Quit`, `Launch(Cube | Model(path) | Image(path))`). It has no terminal I/O, so it is unit-tested directly. `app::run_menu` is the thin loop around it: it owns **one** `TermGuard` for the whole session and calls the same viewer loops (`run_model_loop`, `image_loop`) the non-menu path uses, so `q`/Esc in a viewer returns to the menu. After a viewer returns, the menu creates a fresh `Screen` to force a full redraw.
- A failed launch (bad `.obj`, unreadable PNG/texture) comes back as `Err(String)` from `app::launch` and is shown on the menu's message line (red, cleared on the next key). It never exits the program.
- Entry points (`main.rs`): bare `cubicle` -> menu with **saved** settings. `--menu [flags]` -> menu whose starting settings are the flags (the saved file is not read, but it is still written when settings change). Any other invocation (a FILE, `--cube`, `--once`, or flags without a FILE) behaves as before the menu existed. **Behaviour change:** bare `cubicle` used to spin the cube; use `--cube` for that. The pty tests use `--cube` accordingly.
- Layout constants: list screens have 3 header rows and 2 footer rows (context line + key help); the browser has 2 header rows and 1 footer row. Taps are mapped with the same constants as drawing (`HEADER`, `browser::HEADER_ROWS`), so change them together. Every draw path clips to the terminal width/height (test: `drawing_always_fits_the_terminal`).
- Settings screen rows are indexed (`ROW_TEXTURE = 7`, `ROW_BACK = 8`); `Menu::change`, `setting_line` and `setting_help` must stay in step when adding a row.

### Settings (`src/settings.rs`)
- `Settings` (backend, mode, palette, colour depth or auto, smooth, two_sided, auto_spin, texture, browse_dir) <-> `Config` via `from_config` / `apply`. A tiny lenient `key=value` format (`to_text` / `from_text`); unknown keys and bad values are ignored. Location: `$CUBICLE_CONFIG`, else `$XDG_CONFIG_HOME/cubicle/settings.conf`, else `~/.config/cubicle/settings.conf`. `save_to` / `load_from` take explicit paths so tests never touch the environment; `tests/pty.rs` gives each session its own `CUBICLE_CONFIG`.
- Saved after **every** change. Save failures show a message, never abort.

### File browser (`src/browser.rs`)
- Pure state + drawing. Directories first, then files matching the extension filter (`obj` or `png`, case-insensitive), `..` first, dot-files hidden until `h`. Broken symlinks and unreadable directories are handled (an error line is shown). Selecting a file in the browser returns `Pick::Chosen(path)`.
- Touch: first tap selects, tapping the selected row activates (so a stray tap cannot launch). Row 0 (the title bar) goes back.
- The model browser starts in the remembered `browse_dir`, else `./models`, else `.`. The image and texture browsers start in `./examples` (texture: the previous texture's folder) when no folder is remembered.

### Backends (`src/engine.rs`, `src/gpu.rs`, `src/gpu.wgsl`)
- `settings::Backend` = `Cpu | Gpu | Both`. `Engine::render` is the single entry point used by `ModelView::frame_rows`. It returns an `Outcome` saying what actually drew the frame and, if the CPU stood in for a requested GPU, why. The HUD shows `cpu*` in that case.
- **`Both` definition (a judgement call, see §6):** the canvas is split at `(rows / 2)` terminal rows. The GPU renders the top band on a scoped thread while the CPU renders the bottom band (`RenderOpts::y_range`, which clips the CPU fill to a row range while the projection still uses the whole canvas). The GPU band is then copied into the canvas. Both halves are exactly aligned (tests: `row_ranges_partition_a_frame_exactly`, `gpu_band_lines_up_with_cpu_band`).
- **GPU is an optional cargo feature** (`--features gpu`) pulling in `wgpu 0.19`, `pollster`, `bytemuck`, `libc`. Without the feature `gpu.rs` compiles a stub whose every call returns "this build has no GPU support", so the rest of the code needs no `cfg`s. The default build and the 1.75 MSRV job are unaffected.
- GPU pipeline: vertices are uploaded once per mesh (keyed by pointer + triangle count; re-uploaded when the mesh changes). The vertex shader applies the same rotation matrix, camera and light as the CPU (`raster::Projection` and `raster::light_position` are shared). It outputs clip coordinates with `w = z` so hardware attribute interpolation is perspective-correct, and a depth value that is linear in screen space. The fragment shader reproduces the lighting formula, smooth/flat normals, two-sided lighting (`front_facing`) and bilinear wrapping texture lookup, and writes `(colour, brightness)` to an `Rgba32Float` target (brightness `-1` = empty). The frame is read back and merged into the `Canvas`; everything after that (ASCII/colour/blocks encoding) is shared with the CPU.
- Parity tests (`src/gpu.rs`, feature `gpu`) render the cube, the sphere and a textured cube through both paths and require coverage mismatch < 4 %, mean brightness error < 0.03 and mean colour error < 0.02. In practice the measured mismatch was 0.0000 on cube/sphere.
- Robustness: GPU init is lazy and cached; failure is sticky (`Engine.gpu_error`), so a missing GPU costs one probe, not one per frame. A GPU error mid-run drops the GPU and redraws that frame on the CPU. Driver chatter on stderr during init (Mesa without `XDG_RUNTIME_DIR` prints a message) would corrupt the TUI, so on unix `gpu::quiet` redirects fd 2 to `/dev/null` around adapter/device creation.

### Dependency pins (important when touching `Cargo.lock`)
The newest `indexmap` / `hashbrown` need Rust's `edition2024`, which cargo 1.75 cannot parse. `Cargo.lock` therefore pins `indexmap 2.7.1` and `hashbrown 0.15.5`. If you run `cargo update` with a newer toolchain and then build on 1.75, expect `feature edition2024 is required`; fix with `cargo update -p indexmap --precise 2.7.1`. The optional GPU crates are in the lockfile even when the feature is off, but are not downloaded or compiled for the default build.

### Testing the GPU here
The development sandbox had no GPU, only Mesa's software Vulkan driver (llvmpipe / lavapipe; `apt install mesa-vulkan-drivers`). That is a real Vulkan adapter, so the parity tests exercise the whole pipeline, but nothing was measured on real GPU hardware and **no performance claim is made**. Readback every frame (`map_async` + `poll(Wait)`) is the likely cost on real devices; a double-buffered readback would hide latency (see next steps).

### Known limitations / next steps for this area
1. Not tested on a real GPU, on Termux/Android Vulkan, or on Windows/macOS. CI's `gpu-build` job only proves it compiles and that the tests skip cleanly.
2. `Both` is a fixed 50/50 row split. Auto-balancing by measured frame times would use the GPU's strength when it is faster.
3. Per-frame readback with a blocking `poll`. Consider two staging buffers and presenting the previous frame.
4. Textures are uploaded without mip-maps; there is no `--texture-filter` option.
5. The browser lists a single folder; there is no type-to-search and no preview of the highlighted model.
6. The menu has no "about" or help screen; key help is a one-line footer.
7. Settings are one global file; there are no per-model overrides.

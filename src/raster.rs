//! CPU rasteriser: perspective projection, barycentric fill, 1/z depth buffer,
//! per-pixel point lighting, optional smooth (interpolated) normals.

use crate::display::Canvas;
use crate::image::Image;
use crate::mesh::Mesh;
use crate::vec3::V3;

#[derive(Clone, Copy, Debug)]
pub struct RenderOpts {
    /// Interpolate per-corner normals instead of using the flat face normal.
    pub smooth: bool,
    /// Draw back faces too (lit from their own side) instead of culling them.
    pub two_sided: bool,
    /// Canvas pixels per terminal row: 1 for ASCII, 2 for half-blocks.
    pub pixels_per_row: usize,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts {
            smooth: true,
            two_sided: false,
            pixels_per_row: 1,
        }
    }
}

/// A canvas plus a depth buffer (stores 1/z; larger is closer).
pub struct Frame {
    pub canvas: Canvas,
    depth: Vec<f32>,
}

/// One projected triangle, ready to fill.
struct Prim {
    p: [(f32, f32); 3],
    inv_z: [f32; 3],
    view: [V3; 3],
    normals: [V3; 3],
    flat: V3,
    smooth: bool,
    color: [f32; 3],
    uv: Option<[[f32; 2]; 3]>,
}

impl Frame {
    pub fn new(w: usize, h: usize) -> Self {
        Frame {
            canvas: Canvas::new(w, h, true),
            depth: vec![0.0; w * h],
        }
    }

    fn fill(&mut self, t: &Prim, light: V3, tex: Option<&Image>) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let edge = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
            (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
        };
        let p = t.p;
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-6 {
            return;
        }
        let min_x = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0);
        let max_x = p
            .iter()
            .map(|q| q.0)
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(w as f32 - 1.0);
        let min_y = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0);
        let max_y = p
            .iter()
            .map(|q| q.1)
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(h as f32 - 1.0);
        if max_x < min_x || max_y < min_y {
            return;
        }
        const EPS: f32 = -1e-4; // tolerance so shared edges never leave holes
        for y in (min_y as usize)..=(max_y as usize) {
            for x in (min_x as usize)..=(max_x as usize) {
                let pt = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(p[1], p[2], pt) / area;
                let w1 = edge(p[2], p[0], pt) / area;
                let w2 = edge(p[0], p[1], pt) / area;
                if w0 < EPS || w1 < EPS || w2 < EPS {
                    continue;
                }
                let z = w0 * t.inv_z[0] + w1 * t.inv_z[1] + w2 * t.inv_z[2];
                let i = y * w + x;
                if z <= self.depth[i] {
                    continue;
                }
                self.depth[i] = z;
                // Perspective-correct weights for this pixel.
                let (a0, a1, a2) = (w0 * t.inv_z[0], w1 * t.inv_z[1], w2 * t.inv_z[2]);
                let pos = t.view[0]
                    .scale(a0)
                    .add(t.view[1].scale(a1))
                    .add(t.view[2].scale(a2))
                    .scale(1.0 / (a0 + a1 + a2));
                let n = if t.smooth {
                    let s = t.normals[0]
                        .scale(a0)
                        .add(t.normals[1].scale(a1))
                        .add(t.normals[2].scale(a2));
                    if s.length() > 1e-6 {
                        s.normalize()
                    } else {
                        t.flat
                    }
                } else {
                    t.flat
                };
                let to_light = light.sub(pos);
                let d2 = to_light.dot(to_light);
                let diff = n.dot(to_light.normalize()).max(0.0);
                let atten = 1.0 / (1.0 + 0.06 * d2);
                self.canvas.lum[i] = (0.08 + 1.9 * diff * atten).min(1.0);
                self.canvas.base[i] = match (tex, t.uv) {
                    (Some(img), Some(uv)) => {
                        // Perspective-correct texture coordinates.
                        let s = a0 + a1 + a2;
                        img.sample(
                            (uv[0][0] * a0 + uv[1][0] * a1 + uv[2][0] * a2) / s,
                            (uv[0][1] * a0 + uv[1][1] * a1 + uv[2][1] * a2) / s,
                        )
                    }
                    _ => t.color,
                };
            }
        }
    }
}

/// Draws the mesh into the frame. `ax`/`ay` rotate about the x/y axes and `cam`
/// is the camera distance. The frame must be cleared (new).
///
/// The camera sits at `(0, 0, +cam)` looking toward -z with x to the right and y
/// up, the usual right-handed setup, so models and textures are not mirrored
/// (counter-clockwise triangles face the viewer). The angles are applied
/// negated so that "positive angle" still turns the near face left/up, as the
/// controls expect.
pub fn render(mesh: &Mesh, ax: f32, ay: f32, cam: f32, opts: RenderOpts, frame: &mut Frame) {
    render_textured(mesh, ax, ay, cam, opts, None, frame);
}

/// Like [`render`], but triangles that carry texture coordinates are coloured
/// from `tex` (others keep their own colour).
pub fn render_textured(
    mesh: &Mesh,
    ax: f32,
    ay: f32,
    cam: f32,
    opts: RenderOpts,
    tex: Option<&Image>,
    frame: &mut Frame,
) {
    if frame.canvas.w == 0 || frame.canvas.h == 0 {
        return;
    }
    let ppr = opts.pixels_per_row.max(1) as f32;
    let eye = V3::new(0.0, 0.0, cam);
    let light = V3::new(2.5, 3.0, cam - 0.5);
    // Terminal cells are ~2x taller than wide. Size in cell units so the model
    // fits tall and wide screens, then convert to pixels for the active mode.
    let rows = frame.canvas.h as f32 / ppr;
    let cols = frame.canvas.w as f32;
    let focal_cells = (rows * 1.1).min(cols * 0.65) * cam / 5.0;
    let focal = focal_cells * ppr; // vertical pixels per unit at z = 1
    let x_scale = focal_cells * 2.0; // horizontal pixels per unit at z = 1
    let (cx, cy) = (cols / 2.0, frame.canvas.h as f32 / 2.0);
    let rot = |v: V3| v.rot_y(-ay).rot_x(-ax);

    for tri in &mesh.tris {
        let t = [rot(tri.v[0]), rot(tri.v[1]), rot(tri.v[2])];
        let mut normals = [rot(tri.n[0]), rot(tri.n[1]), rot(tri.n[2])];
        let mut flat = t[1].sub(t[0]).cross(t[2].sub(t[0])).normalize();
        if flat.dot(eye.sub(t[0])) <= 0.0 {
            if !opts.two_sided {
                continue; // backface culling
            }
            flat = flat.neg();
            normals = [normals[0].neg(), normals[1].neg(), normals[2].neg()];
        }
        let mut p = [(0.0, 0.0); 3];
        let mut inv_z = [0.0; 3];
        let mut behind = false;
        for (i, v) in t.iter().enumerate() {
            let z = cam - v.z;
            if z <= 0.1 {
                behind = true;
                break;
            }
            p[i] = (cx + v.x * x_scale / z, cy - v.y * focal / z);
            inv_z[i] = 1.0 / z;
        }
        if behind {
            continue;
        }
        let prim = Prim {
            p,
            inv_z,
            view: t,
            normals,
            flat,
            smooth: opts.smooth,
            color: tri.color,
            uv: tri.uv,
        };
        frame.fill(&prim, light, tex);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{Mode, Palette};

    fn covered(f: &Frame) -> usize {
        f.canvas.lum.iter().filter(|&&l| l >= 0.0).count()
    }

    #[test]
    fn cube_draws_something_and_stays_centred() {
        let mut f = Frame::new(60, 24);
        render(&Mesh::cube(), 0.4, 0.6, 5.0, RenderOpts::default(), &mut f);
        let n = covered(&f);
        assert!(n > 100 && n < 60 * 24, "covered {n}");
        // Centre pixel is on the cube, corners are not.
        assert!(f.canvas.lum[12 * 60 + 30] >= 0.0);
        assert!(f.canvas.lum[0] < 0.0 && f.canvas.lum[60 * 24 - 1] < 0.0);
    }

    #[test]
    fn zoom_in_covers_more_pixels() {
        let (mut near, mut far) = (Frame::new(60, 24), Frame::new(60, 24));
        render(&Mesh::cube(), 0.4, 0.6, 3.0, RenderOpts::default(), &mut near);
        render(&Mesh::cube(), 0.4, 0.6, 9.0, RenderOpts::default(), &mut far);
        assert!(covered(&near) > covered(&far));
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (1, 30), (200, 1), (3, 2)] {
            let mut f = Frame::new(w, h);
            render(&Mesh::cube(), 1.0, 2.0, 5.0, RenderOpts::default(), &mut f);
            let _ = f.canvas.encode(Mode::Blocks, Palette::Natural);
        }
    }

    #[test]
    fn two_sided_shows_back_faces_of_open_meshes() {
        // A single triangle facing away from the camera.
        use crate::mesh::RawTri;
        let raw = vec![RawTri::plain([
            V3::new(-1.0, -1.0, 0.0),
            V3::new(0.0, 1.0, 0.0),
            V3::new(1.0, -1.0, 0.0),
        ])];
        let mesh = Mesh::from_raw(raw, 60.0).unwrap();
        let count = |two_sided: bool| {
            let mut a = Frame::new(40, 20);
            let mut b = Frame::new(40, 20);
            let o = RenderOpts {
                two_sided,
                ..RenderOpts::default()
            };
            render(&mesh, 0.0, 0.0, 5.0, o, &mut a);
            render(&mesh, 0.0, std::f32::consts::PI, 5.0, o, &mut b);
            (covered(&a), covered(&b))
        };
        let (a0, b0) = count(false);
        let (a1, b1) = count(true);
        assert!(a0 == 0 || b0 == 0, "culling hides exactly one side");
        assert!(a1 > 0 && b1 > 0, "two-sided shows both");
    }

    #[test]
    fn view_is_not_mirrored() {
        // Right angle at the lower-left, counter-clockwise seen from the camera.
        use crate::mesh::RawTri;
        let raw = vec![RawTri::plain([
            V3::new(-1.0, -1.0, 0.0),
            V3::new(1.0, -1.0, 0.0),
            V3::new(-1.0, 1.0, 0.0),
        ])];
        let mesh = Mesh::from_raw(raw, 60.0).unwrap();
        let mut f = Frame::new(40, 20);
        render(&mesh, 0.0, 0.0, 5.0, RenderOpts::default(), &mut f);
        let at = |x: usize, y: usize| f.canvas.lum[y * 40 + x] >= 0.0;
        assert!(at(14, 12), "lower-left should be filled");
        assert!(!at(26, 7), "upper-right should be empty");
    }

    #[test]
    fn texture_colours_cube_pixels() {
        // A 1x1 pure-blue texture must turn every covered pixel blue.
        let tex = Image {
            w: 1,
            h: 1,
            rgb: vec![[0, 0, 255]],
        };
        let mut f = Frame::new(60, 24);
        render_textured(
            &Mesh::cube(),
            0.4,
            0.6,
            5.0,
            RenderOpts::default(),
            Some(&tex),
            &mut f,
        );
        let mut n = 0;
        for (l, b) in f.canvas.lum.iter().zip(&f.canvas.base) {
            if *l >= 0.0 {
                assert_eq!(*b, [0.0, 0.0, 1.0]);
                n += 1;
            }
        }
        assert!(n > 100);
    }

    #[test]
    fn cube_texture_is_not_mirrored_on_any_face() {
        // Every cube triangle is counter-clockwise seen from outside, so an
        // un-mirrored mapping (u right, v up) must also be counter-clockwise in uv space.
        for t in &Mesh::cube().tris {
            let uv = t.uv.expect("cube has uvs");
            let area =
                (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1]) - (uv[1][1] - uv[0][1]) * (uv[2][0] - uv[0][0]);
            assert!(area > 1e-6, "mirrored or degenerate uv: {uv:?}");
        }
    }

    #[test]
    fn blocks_mode_keeps_the_same_physical_size() {
        let mut ascii = Frame::new(80, 20);
        let mut blocks = Frame::new(80, 40);
        render(&Mesh::cube(), 0.4, 0.6, 5.0, RenderOpts::default(), &mut ascii);
        let o = RenderOpts {
            pixels_per_row: 2,
            ..RenderOpts::default()
        };
        render(&Mesh::cube(), 0.4, 0.6, 5.0, o, &mut blocks);
        // Twice the vertical pixels -> roughly twice the covered pixels.
        let ratio = covered(&blocks) as f32 / covered(&ascii) as f32;
        assert!((ratio - 2.0).abs() < 0.3, "ratio {ratio}");
    }
}

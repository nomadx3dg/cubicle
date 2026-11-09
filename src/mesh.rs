//! Triangle meshes: construction, normalisation and normal generation.

use crate::vec3::V3;
use std::collections::HashMap;

/// Every mesh is scaled so its bounding sphere has this radius. It equals the
/// circumradius of the built-in cube (sqrt 3), so all models frame alike.
pub const TARGET_RADIUS: f32 = 1.732;

/// Default crease angle (degrees). Edges sharper than this stay hard when
/// smooth shading is on, so cubes stay crisp while spheres go smooth.
pub const DEFAULT_CREASE_DEG: f32 = 60.0;

const DEFAULT_COLOR: [f32; 3] = [0.85, 0.85, 0.85];

/// A triangle ready to render. `n` are per-corner (smooth) normals.
#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub v: [V3; 3],
    pub n: [V3; 3],
    /// Linear RGB in 0..1, multiplied by the lighting when drawn in colour.
    pub color: [f32; 3],
}

/// Un-normalised input triangle, as produced by loaders.
#[derive(Clone, Copy, Debug)]
pub struct RawTri {
    pub v: [V3; 3],
    /// Normals supplied by the file (`vn`), if all three corners had one.
    pub n: Option<[V3; 3]>,
    pub color: [f32; 3],
}

impl RawTri {
    pub fn plain(v: [V3; 3]) -> Self {
        RawTri {
            v,
            n: None,
            color: DEFAULT_COLOR,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub tris: Vec<Triangle>,
}

fn raw_cross(v: &[V3; 3]) -> V3 {
    v[1].sub(v[0]).cross(v[2].sub(v[0]))
}

impl Mesh {
    /// Centres the mesh on the origin, scales it to [`TARGET_RADIUS`], repairs
    /// inverted winding, and fills in smooth normals the file did not supply.
    pub fn from_raw(raw: Vec<RawTri>, crease_deg: f32) -> Result<Mesh, String> {
        let mut raw: Vec<RawTri> = raw
            .into_iter()
            .filter(|t| t.v.iter().all(|p| p.is_finite()) && raw_cross(&t.v).length() > 1e-12)
            .collect();
        if raw.is_empty() {
            return Err("model contains no usable triangles".into());
        }

        // Centre on the bounding-box centre and scale the bounding sphere.
        let (mut lo, mut hi) = (
            V3::new(f32::MAX, f32::MAX, f32::MAX),
            V3::new(f32::MIN, f32::MIN, f32::MIN),
        );
        for p in raw.iter().flat_map(|t| t.v.iter()) {
            lo = V3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = V3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        let centre = lo.add(hi).scale(0.5);
        let radius = raw
            .iter()
            .flat_map(|t| t.v.iter())
            .map(|p| p.sub(centre).length())
            .fold(0.0_f32, f32::max);
        if radius <= 0.0 {
            return Err("model has zero size".into());
        }
        let k = TARGET_RADIUS / radius;
        for t in &mut raw {
            for p in &mut t.v {
                *p = p.sub(centre).scale(k);
            }
        }

        // Signed volume < 0 means the faces wind clockwise (inside-out).
        let volume: f32 = raw.iter().map(|t| t.v[0].dot(t.v[1].cross(t.v[2]))).sum::<f32>() / 6.0;
        if volume < 0.0 {
            for t in &mut raw {
                t.v.swap(1, 2);
                if let Some(n) = &mut t.n {
                    n.swap(1, 2);
                }
            }
        }

        let computed = smooth_normals(&raw, crease_deg);
        let tris = raw
            .iter()
            .zip(computed)
            .map(|(t, auto)| Triangle {
                v: t.v,
                n: match t.n {
                    Some(n) => [n[0].normalize(), n[1].normalize(), n[2].normalize()],
                    None => auto,
                },
                color: t.color,
            })
            .collect();
        Ok(Mesh { tris })
    }

    /// The built-in cube: 12 triangles, a different colour per face.
    pub fn cube() -> Mesh {
        let v = [
            V3::new(-1.0, -1.0, -1.0),
            V3::new(1.0, -1.0, -1.0),
            V3::new(1.0, 1.0, -1.0),
            V3::new(-1.0, 1.0, -1.0),
            V3::new(-1.0, -1.0, 1.0),
            V3::new(1.0, -1.0, 1.0),
            V3::new(1.0, 1.0, 1.0),
            V3::new(-1.0, 1.0, 1.0),
        ];
        // Each face is a quad listed around its perimeter.
        let quads = [
            ([0, 1, 2, 3], [0.95, 0.35, 0.35]),
            ([4, 5, 6, 7], [0.35, 0.85, 0.45]),
            ([0, 3, 7, 4], [0.40, 0.55, 1.00]),
            ([1, 5, 6, 2], [1.00, 0.85, 0.30]),
            ([0, 4, 5, 1], [0.85, 0.45, 0.95]),
            ([3, 2, 6, 7], [0.30, 0.90, 0.90]),
        ];
        let mut raw = Vec::new();
        for (q, color) in quads {
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                let tri = orient_outward([v[t[0]], v[t[1]], v[t[2]]]);
                raw.push(RawTri {
                    v: tri,
                    n: None,
                    color,
                });
            }
        }
        // Already centred with the right radius; from_raw leaves it unchanged.
        Mesh::from_raw(raw, DEFAULT_CREASE_DEG).expect("cube is valid")
    }
}

/// For a mesh centred on the origin, flip any triangle whose normal faces inward.
fn orient_outward(t: [V3; 3]) -> [V3; 3] {
    let n = raw_cross(&t);
    let centroid = t[0].add(t[1]).add(t[2]).scale(1.0 / 3.0);
    if n.dot(centroid) < 0.0 {
        [t[0], t[2], t[1]]
    } else {
        t
    }
}

fn weld_key(p: V3) -> (i64, i64, i64) {
    let q = |f: f32| (f * 10_000.0).round() as i64;
    (q(p.x), q(p.y), q(p.z))
}

/// Per-corner normals: each corner averages (area-weighted) the faces that share
/// its position and lie within `crease_deg` of its own face.
pub fn smooth_normals(tris: &[RawTri], crease_deg: f32) -> Vec<[V3; 3]> {
    let cos_limit = crease_deg.to_radians().cos();
    let crosses: Vec<V3> = tris.iter().map(|t| raw_cross(&t.v)).collect();
    let units: Vec<V3> = crosses.iter().map(|c| c.normalize()).collect();

    let mut groups: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for p in &t.v {
            let g = groups.entry(weld_key(*p)).or_default();
            if g.last() != Some(&i) {
                g.push(i);
            }
        }
    }

    tris.iter()
        .enumerate()
        .map(|(i, t)| {
            let mut out = [units[i]; 3];
            for (c, p) in t.v.iter().enumerate() {
                let mut sum = V3::default();
                for &j in &groups[&weld_key(*p)] {
                    if units[j].dot(units[i]) >= cos_limit {
                        sum = sum.add(crosses[j]);
                    }
                }
                if sum.length() > 0.0 {
                    out[c] = sum.normalize();
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_has_twelve_outward_triangles() {
        let m = Mesh::cube();
        assert_eq!(m.tris.len(), 12);
        for t in &m.tris {
            let n = raw_cross(&t.v);
            let c = t.v[0].add(t.v[1]).add(t.v[2]).scale(1.0 / 3.0);
            assert!(n.dot(c) > 0.0);
        }
    }

    #[test]
    fn cube_stays_flat_with_smoothing() {
        // 90 degree edges exceed the 60 degree crease, so normals are face normals.
        for t in &Mesh::cube().tris {
            let face = raw_cross(&t.v).normalize();
            for n in t.n {
                assert!(n.dot(face) > 0.999);
            }
        }
    }

    fn tetra(flip: bool) -> Vec<RawTri> {
        let p = [
            V3::new(1.0, 1.0, 1.0),
            V3::new(-1.0, -1.0, 1.0),
            V3::new(-1.0, 1.0, -1.0),
            V3::new(1.0, -1.0, -1.0),
        ];
        let faces = [[0, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]];
        faces
            .iter()
            .map(|f| {
                let mut v = [p[f[0]], p[f[1]], p[f[2]]];
                if flip {
                    v.swap(1, 2);
                }
                RawTri::plain(v)
            })
            .collect()
    }

    #[test]
    fn inside_out_winding_is_repaired() {
        for flip in [false, true] {
            let m = Mesh::from_raw(tetra(flip), 60.0).unwrap();
            let vol: f32 = m.tris.iter().map(|t| t.v[0].dot(t.v[1].cross(t.v[2]))).sum();
            assert!(vol > 0.0, "flip={flip}");
        }
    }

    #[test]
    fn mesh_is_centred_and_scaled() {
        let shifted: Vec<RawTri> = tetra(false)
            .into_iter()
            .map(|mut t| {
                for p in &mut t.v {
                    *p = p.scale(5.0).add(V3::new(10.0, -3.0, 7.0));
                }
                t
            })
            .collect();
        let m = Mesh::from_raw(shifted, 60.0).unwrap();
        let r = m
            .tris
            .iter()
            .flat_map(|t| t.v)
            .map(|p| p.length())
            .fold(0.0, f32::max);
        assert!((r - TARGET_RADIUS).abs() < 1e-3);
    }

    #[test]
    fn smoothing_respects_crease() {
        // Two triangles sharing an edge at 90 degrees: hard at 60, merged at 120.
        let a = RawTri::plain([
            V3::new(0.0, 0.0, 0.0),
            V3::new(1.0, 0.0, 0.0),
            V3::new(0.0, 1.0, 0.0),
        ]);
        let b = RawTri::plain([
            V3::new(0.0, 0.0, 0.0),
            V3::new(0.0, 0.0, 1.0),
            V3::new(1.0, 0.0, 0.0),
        ]);
        let hard = smooth_normals(&[a, b], 60.0);
        let soft = smooth_normals(&[a, b], 120.0);
        assert!(hard[0][0].dot(hard[1][0]) < 0.01);
        assert!(soft[0][0].dot(soft[1][0]) > 0.99);
    }

    #[test]
    fn empty_and_degenerate_meshes_are_rejected() {
        assert!(Mesh::from_raw(vec![], 60.0).is_err());
        let p = V3::new(1.0, 1.0, 1.0);
        assert!(Mesh::from_raw(vec![RawTri::plain([p, p, p])], 60.0).is_err());
    }
}

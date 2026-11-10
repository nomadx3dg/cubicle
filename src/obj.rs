//! Wavefront `.obj` loader.
//!
//! Supported: `v` (optionally with `r g b` vertex colours), `vn`, and `f` in all
//! four index forms (`v`, `v/vt`, `v//vn`, `v/vt/vn`), including negative
//! (relative) indices. Polygons are fan-triangulated, so they must be convex.
//! Texture coordinates (`vt`) are kept when a face supplies all three corners.
//! Ignored: groups, objects, smoothing groups, materials.

use crate::mesh::{Mesh, RawTri};
use crate::vec3::V3;
use std::path::Path;

const DEFAULT_COLOR: [f32; 3] = [0.85, 0.85, 0.85];

fn floats<'a>(it: impl Iterator<Item = &'a str>, line: usize) -> Result<Vec<f32>, String> {
    it.map(|s| {
        s.parse::<f32>()
            .map_err(|_| format!("line {line}: bad number '{s}'"))
    })
    .collect()
}

/// Resolves a 1-based or negative OBJ index against `len` items.
fn resolve(tok: &str, len: usize, line: usize) -> Result<usize, String> {
    let i: i64 = tok
        .parse()
        .map_err(|_| format!("line {line}: bad index '{tok}'"))?;
    let idx = match i.cmp(&0) {
        std::cmp::Ordering::Greater => i - 1,
        std::cmp::Ordering::Less => len as i64 + i,
        std::cmp::Ordering::Equal => -1,
    };
    if idx < 0 || idx as usize >= len {
        return Err(format!("line {line}: index {i} out of range (have {len})"));
    }
    Ok(idx as usize)
}

pub fn parse(src: &str) -> Result<Vec<RawTri>, String> {
    let mut pos: Vec<V3> = Vec::new();
    let mut vcol: Vec<Option<[f32; 3]>> = Vec::new();
    let mut nor: Vec<V3> = Vec::new();
    let mut tex: Vec<[f32; 2]> = Vec::new();
    let mut out: Vec<RawTri> = Vec::new();

    for (i, line) in src.lines().enumerate() {
        let ln = i + 1;
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        let Some(tag) = it.next() else { continue };
        match tag {
            "v" => {
                let f = floats(it, ln)?;
                if f.len() < 3 {
                    return Err(format!("line {ln}: 'v' needs 3 coordinates"));
                }
                pos.push(V3::new(f[0], f[1], f[2]));
                vcol.push(if f.len() >= 6 {
                    Some([f[3].clamp(0.0, 1.0), f[4].clamp(0.0, 1.0), f[5].clamp(0.0, 1.0)])
                } else {
                    None
                });
            }
            "vn" => {
                let f = floats(it, ln)?;
                if f.len() < 3 {
                    return Err(format!("line {ln}: 'vn' needs 3 components"));
                }
                nor.push(V3::new(f[0], f[1], f[2]));
            }
            "vt" => {
                let f = floats(it, ln)?;
                if f.len() < 2 {
                    return Err(format!("line {ln}: 'vt' needs at least 2 components"));
                }
                tex.push([f[0], f[1]]);
            }
            "f" => {
                // (position, texcoord, normal) indices
                type Corner = (usize, Option<usize>, Option<usize>);
                let mut corners: Vec<Corner> = Vec::new();
                for tok in it {
                    let mut parts = tok.split('/');
                    let p = resolve(parts.next().unwrap_or(""), pos.len(), ln)?;
                    let t = match parts.next() {
                        Some(s) if !s.is_empty() => Some(resolve(s, tex.len(), ln)?),
                        _ => None,
                    };
                    let n = match parts.next() {
                        Some(s) if !s.is_empty() => Some(resolve(s, nor.len(), ln)?),
                        _ => None,
                    };
                    corners.push((p, t, n));
                }
                if corners.len() < 3 {
                    return Err(format!("line {ln}: face needs at least 3 vertices"));
                }
                for k in 1..corners.len() - 1 {
                    let c = [corners[0], corners[k], corners[k + 1]];
                    let v = [pos[c[0].0], pos[c[1].0], pos[c[2].0]];
                    let n = match (c[0].2, c[1].2, c[2].2) {
                        (Some(a), Some(b), Some(d)) => Some([nor[a], nor[b], nor[d]]),
                        _ => None,
                    };
                    let color = match (vcol[c[0].0], vcol[c[1].0], vcol[c[2].0]) {
                        (Some(a), Some(b), Some(d)) => [
                            (a[0] + b[0] + d[0]) / 3.0,
                            (a[1] + b[1] + d[1]) / 3.0,
                            (a[2] + b[2] + d[2]) / 3.0,
                        ],
                        _ => DEFAULT_COLOR,
                    };
                    let uv = match (c[0].1, c[1].1, c[2].1) {
                        (Some(a), Some(b), Some(d)) => Some([tex[a], tex[b], tex[d]]),
                        _ => None,
                    };
                    out.push(RawTri { v, n, color, uv });
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        return Err("no faces found in .obj file".into());
    }
    Ok(out)
}

pub fn load(path: &Path, crease_deg: f32) -> Result<Mesh, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let raw = parse(&src).map_err(|e| format!("{}: {e}", path.display()))?;
    Mesh::from_raw(raw, crease_deg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_triangle() {
        let t = parse("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        assert_eq!(t.len(), 1);
        assert!(t[0].n.is_none());
    }

    #[test]
    fn quads_are_fan_triangulated() {
        let t = parse("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n").unwrap();
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn negative_indices_and_all_index_forms() {
        let src = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nvt 0 0\n\
                   f -3//-1 -2//1 -1//1\nf 1/1/1 2/1/1 3/1/1\nf 1/1 2/1 3/1\n";
        let t = parse(src).unwrap();
        assert_eq!(t.len(), 3);
        assert!(t[0].n.is_some() && t[1].n.is_some());
        assert!(t[2].n.is_none());
    }

    #[test]
    fn texture_coordinates_are_kept() {
        let src = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\nf 1 2 3\n";
        let t = parse(src).unwrap();
        assert_eq!(t[0].uv, Some([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]));
        assert_eq!(t[1].uv, None);
        assert!(parse("v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nf 1/2 2/1 3/1\n").is_err());
    }

    #[test]
    fn comments_blank_lines_and_crlf() {
        let t = parse("# hi\r\n\r\nv 0 0 0 # origin\r\nv 1 0 0\r\nv 0 1 0\r\nf 1 2 3 # tri\r\n").unwrap();
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn vertex_colours_are_averaged() {
        let t = parse("v 0 0 0 1 0 0\nv 1 0 0 1 0 0\nv 0 1 0 1 0 0\nf 1 2 3\n").unwrap();
        assert_eq!(t[0].color, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn errors_carry_line_numbers() {
        let e = parse("v 0 0 0\nf 1 2 3\n").unwrap_err();
        assert!(e.contains("line 2"), "{e}");
        let e = parse("v 0 0 x\n").unwrap_err();
        assert!(e.contains("line 1"), "{e}");
        assert!(parse("v 0 0 0\nv 1 0 0\nf 0 1 2\n").is_err());
        assert!(parse("v 0 0 0\n").is_err());
    }
}

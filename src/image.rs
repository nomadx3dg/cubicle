//! PNG loading and conversion to a [`Canvas`] for ASCII / block display.

use crate::display::Canvas;
use std::path::Path;

/// An 8-bit RGB image. Transparency is composited onto black.
#[derive(Clone, Debug)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[u8; 3]>,
}

impl Image {
    pub fn load(path: &Path) -> Result<Image, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::decode(std::io::BufReader::new(file)).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn decode<R: std::io::Read>(r: R) -> Result<Image, String> {
        let mut dec = png::Decoder::new(r);
        // Expand palettes / low bit depths and reduce 16-bit to 8-bit.
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| format!("not a valid PNG ({e})"))?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("PNG decode failed ({e})"))?;
        let (w, h) = (info.width as usize, info.height as usize);
        if w == 0 || h == 0 {
            return Err("image is empty".into());
        }
        let data = &buf[..info.buffer_size()];
        let blend = |c: u8, a: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        let rgb: Vec<[u8; 3]> = match info.color_type {
            png::ColorType::Grayscale => data.iter().map(|&g| [g, g, g]).collect(),
            png::ColorType::GrayscaleAlpha => data.chunks_exact(2).map(|p| [blend(p[0], p[1]); 3]).collect(),
            png::ColorType::Rgb | png::ColorType::Indexed => {
                data.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect()
            }
            png::ColorType::Rgba => data
                .chunks_exact(4)
                .map(|p| [blend(p[0], p[3]), blend(p[1], p[3]), blend(p[2], p[3])])
                .collect(),
        };
        if rgb.len() != w * h {
            return Err("unexpected PNG pixel layout".into());
        }
        Ok(Image { w, h, rgb })
    }

    /// Fits the image into a `cols` x `rows` terminal area (preserving aspect,
    /// centred) and box-filters it down. `pixels_per_row` is 1 for ASCII and
    /// 2 for half-blocks. `invert` flips glyph brightness for light terminals.
    pub fn to_canvas(&self, cols: usize, rows: usize, pixels_per_row: usize, invert: bool) -> Canvas {
        let ppr = pixels_per_row.max(1);
        let (gw, gh) = (cols, rows * ppr);
        let mut canvas = Canvas::new(gw, gh, false);
        if gw == 0 || gh == 0 {
            return canvas;
        }
        // A canvas pixel is `cell_h` times as tall as it is wide (cells are 1:2).
        let cell_h = 2.0 / ppr as f32;
        let s = (gw as f32 / self.w as f32).min(gh as f32 * cell_h / self.h as f32);
        let ow = ((self.w as f32 * s).round() as usize).clamp(1, gw);
        let oh = ((self.h as f32 * s / cell_h).round() as usize).clamp(1, gh);
        let (ox, oy) = ((gw - ow) / 2, (gh - oh) / 2);

        for y in 0..oh {
            let y0 = y * self.h / oh;
            let y1 = ((y + 1) * self.h).div_ceil(oh).max(y0 + 1).min(self.h);
            for x in 0..ow {
                let x0 = x * self.w / ow;
                let x1 = ((x + 1) * self.w).div_ceil(ow).max(x0 + 1).min(self.w);
                let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
                for yy in y0..y1 {
                    for p in &self.rgb[yy * self.w + x0..yy * self.w + x1] {
                        r += p[0] as u32;
                        g += p[1] as u32;
                        b += p[2] as u32;
                        n += 1;
                    }
                }
                let c = [
                    r as f32 / (n * 255) as f32,
                    g as f32 / (n * 255) as f32,
                    b as f32 / (n * 255) as f32,
                ];
                let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                let i = (oy + y) * gw + ox + x;
                canvas.lum[i] = if invert { 1.0 - luma } else { luma };
                canvas.base[i] = c;
            }
        }
        canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{Mode, Palette};

    /// Encodes raw pixels as a PNG in memory.
    fn make_png(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn decodes_rgb_gray_and_rgba() {
        let rgb = Image::decode(&make_png(1, 1, png::ColorType::Rgb, &[10, 20, 30])[..]).unwrap();
        assert_eq!(rgb.rgb, vec![[10, 20, 30]]);
        let gray = Image::decode(&make_png(2, 1, png::ColorType::Grayscale, &[0, 255])[..]).unwrap();
        assert_eq!(gray.rgb, vec![[0, 0, 0], [255, 255, 255]]);
        // Half-transparent white over black is mid grey.
        let rgba = Image::decode(&make_png(1, 1, png::ColorType::Rgba, &[255, 255, 255, 128])[..]).unwrap();
        assert!((rgba.rgb[0][0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Image::decode(&b"definitely not a png"[..]).is_err());
    }

    fn split_image() -> Image {
        // Left half black, right half white.
        let mut rgb = Vec::new();
        for _ in 0..4 {
            rgb.extend([[0u8; 3], [0; 3], [255; 3], [255; 3]]);
        }
        Image { w: 4, h: 4, rgb }
    }

    #[test]
    fn ascii_conversion_is_dark_then_bright() {
        let rows = split_image()
            .to_canvas(8, 4, 1, false)
            .encode(Mode::Ascii, Palette::Natural);
        let row = &rows[2];
        assert!(row.contains(' ') && row.contains('@'));
        assert!(row.find(' ').unwrap() < row.find('@').unwrap());
    }

    #[test]
    fn invert_flips_brightness() {
        let rows = split_image()
            .to_canvas(8, 4, 1, true)
            .encode(Mode::Ascii, Palette::Natural);
        assert!(rows[2].find('@').unwrap() < rows[2].rfind(' ').unwrap());
    }

    #[test]
    fn aspect_ratio_is_preserved_and_centred() {
        // A wide 8x2 image in a tall area: letterboxed top and bottom.
        let img = Image {
            w: 8,
            h: 2,
            rgb: vec![[255; 3]; 16],
        };
        let c = img.to_canvas(16, 10, 1, false);
        let filled: Vec<usize> = (0..10)
            .filter(|&y| (0..16).any(|x| c.lum[y * 16 + x] >= 0.0))
            .collect();
        // 16 columns wide, cells are twice as tall as wide: 16 * (2/8) / 2 = 2 rows.
        assert_eq!(filled.len(), 2);
        assert_eq!(filled[0], (10 - filled.len()) / 2);
    }

    #[test]
    fn tiny_and_huge_targets_do_not_panic() {
        let img = split_image();
        for (c, r) in [(0, 0), (1, 1), (500, 1), (1, 500)] {
            for ppr in [1, 2] {
                let _ = img.to_canvas(c, r, ppr, false).encode(Mode::Blocks, Palette::Ice);
            }
        }
    }
}

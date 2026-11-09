//! Turning a pixel grid into terminal text: ASCII ramp, coloured ASCII, or
//! half-block truecolor, plus the touch HUD bar.

/// Dark -> bright. Add or swap characters to change the look.
pub const RAMP: &[u8] = b" .,:;-=+*#%@";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Plain characters, no escape codes. Works everywhere.
    Ascii,
    /// Same characters, tinted with truecolor foreground.
    AsciiColor,
    /// Half-block glyphs: two truecolor pixels per cell.
    Blocks,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Ascii => Mode::AsciiColor,
            Mode::AsciiColor => Mode::Blocks,
            Mode::Blocks => Mode::Ascii,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Mode::Ascii => "ascii",
            Mode::AsciiColor => "color",
            Mode::Blocks => "blocks",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "ascii" => Some(Mode::Ascii),
            "color" | "colour" => Some(Mode::AsciiColor),
            "blocks" | "block" => Some(Mode::Blocks),
            _ => None,
        }
    }
    /// Canvas pixels per terminal row.
    pub fn pixels_per_row(self) -> usize {
        if self == Mode::Blocks {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    /// The model's / image's own colours.
    Natural,
    Green,
    Amber,
    Ice,
}

impl Palette {
    pub fn next(self) -> Palette {
        match self {
            Palette::Natural => Palette::Green,
            Palette::Green => Palette::Amber,
            Palette::Amber => Palette::Ice,
            Palette::Ice => Palette::Natural,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Palette::Natural => "natural",
            Palette::Green => "green",
            Palette::Amber => "amber",
            Palette::Ice => "ice",
        }
    }
    pub fn parse(s: &str) -> Option<Palette> {
        match s.to_ascii_lowercase().as_str() {
            "natural" => Some(Palette::Natural),
            "green" => Some(Palette::Green),
            "amber" => Some(Palette::Amber),
            "ice" => Some(Palette::Ice),
            _ => None,
        }
    }
    /// Applies the palette to an unlit colour (linear 0..1).
    pub fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        let tint = match self {
            Palette::Natural => return rgb,
            Palette::Green => [0.35, 1.0, 0.45],
            Palette::Amber => [1.0, 0.70, 0.20],
            Palette::Ice => [0.55, 0.85, 1.0],
        };
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        let k = (luma / 0.85).min(1.0);
        [tint[0] * k, tint[1] * k, tint[2] * k]
    }
}

/// A grid of pixels. `lum < 0` marks an empty pixel.
#[derive(Clone, Debug)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    /// Brightness 0..1 (drives the character choice).
    pub lum: Vec<f32>,
    /// Surface colour. If `lit`, it is multiplied by `lum` when drawn in colour;
    /// otherwise it is the final pixel colour (images).
    pub base: Vec<[f32; 3]>,
    pub lit: bool,
}

impl Canvas {
    pub fn new(w: usize, h: usize, lit: bool) -> Self {
        Canvas {
            w,
            h,
            lum: vec![-1.0; w * h],
            base: vec![[0.0; 3]; w * h],
            lit,
        }
    }

    fn glyph(l: f32) -> char {
        RAMP[(l.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32 + 0.5) as usize] as char
    }

    fn to_u8(c: [f32; 3]) -> [u8; 3] {
        [
            (c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }

    /// Colour for a half-block pixel.
    fn block_rgb(&self, i: usize, palette: Palette) -> Option<[u8; 3]> {
        let l = self.lum[i];
        if l < 0.0 {
            return None;
        }
        let c = palette.apply(self.base[i]);
        // Lift the shadows so unlit faces stay readable instead of going black.
        let k = if self.lit {
            0.22 + 0.78 * l.min(1.0).powf(0.7)
        } else {
            1.0
        };
        Some(Self::to_u8([c[0] * k, c[1] * k, c[2] * k]))
    }

    /// Foreground colour for a coloured ASCII glyph.
    fn glyph_rgb(&self, i: usize, palette: Palette) -> [u8; 3] {
        let c = palette.apply(self.base[i]);
        let k = if self.lit {
            0.45 + 0.55 * self.lum[i].min(1.0)
        } else {
            1.0
        };
        Self::to_u8([c[0] * k, c[1] * k, c[2] * k])
    }

    /// Encodes the canvas as terminal rows. For [`Mode::Blocks`] the canvas
    /// height is two pixels per row (an odd final row is ignored).
    pub fn encode(&self, mode: Mode, palette: Palette) -> Vec<String> {
        match mode {
            Mode::Ascii => (0..self.h)
                .map(|y| {
                    (0..self.w)
                        .map(|x| {
                            let l = self.lum[y * self.w + x];
                            if l < 0.0 {
                                ' '
                            } else {
                                Self::glyph(l)
                            }
                        })
                        .collect()
                })
                .collect(),
            Mode::AsciiColor => (0..self.h)
                .map(|y| {
                    let mut row = RowWriter::default();
                    for x in 0..self.w {
                        let i = y * self.w + x;
                        if self.lum[i] < 0.0 {
                            row.cell(' ', None, None);
                        } else {
                            let g = Self::glyph(self.lum[i]);
                            let fg = if g == ' ' {
                                None
                            } else {
                                Some(self.glyph_rgb(i, palette))
                            };
                            row.cell(g, fg, None);
                        }
                    }
                    row.finish()
                })
                .collect(),
            Mode::Blocks => (0..self.h / 2)
                .map(|r| {
                    let mut row = RowWriter::default();
                    for x in 0..self.w {
                        let top = self.block_rgb(2 * r * self.w + x, palette);
                        let bot = self.block_rgb((2 * r + 1) * self.w + x, palette);
                        match (top, bot) {
                            (None, None) => row.cell(' ', None, None),
                            (Some(t), None) => row.cell('\u{2580}', Some(t), None),
                            (None, Some(b)) => row.cell('\u{2584}', Some(b), None),
                            (Some(t), Some(b)) => row.cell('\u{2580}', Some(t), Some(b)),
                        }
                    }
                    row.finish()
                })
                .collect(),
        }
    }
}

/// Builds one row, emitting colour escapes only when the colour changes.
#[derive(Default)]
struct RowWriter {
    s: String,
    fg: Option<[u8; 3]>,
    bg: Option<[u8; 3]>,
}

impl RowWriter {
    fn cell(&mut self, ch: char, fg: Option<[u8; 3]>, bg: Option<[u8; 3]>) {
        use std::fmt::Write;
        if bg != self.bg {
            match bg {
                Some(c) => {
                    let _ = write!(self.s, "\x1b[48;2;{};{};{}m", c[0], c[1], c[2]);
                }
                None => self.s.push_str("\x1b[49m"),
            }
            self.bg = bg;
        }
        // A space shows no foreground, so don't spend bytes changing it.
        if ch != ' ' && fg != self.fg {
            if let Some(c) = fg {
                let _ = write!(self.s, "\x1b[38;2;{};{};{}m", c[0], c[1], c[2]);
                self.fg = fg;
            }
        }
        self.s.push(ch);
    }

    fn finish(mut self) -> String {
        if self.fg.is_some() || self.bg.is_some() {
            self.s.push_str("\x1b[0m");
        }
        self.s
    }
}

/// Splits `w` columns into `n` button zones; returns each zone's `(start, end)`.
fn zones(w: usize, n: usize) -> Vec<(usize, usize)> {
    (0..n).map(|i| (w * i / n, w * (i + 1) / n)).collect()
}

/// The bottom bar: `labels` laid out as equal-width, centred touch buttons.
pub fn hud(w: usize, labels: &[&str]) -> String {
    let mut out = String::new();
    for ((a, b), label) in zones(w, labels.len()).into_iter().zip(labels) {
        let width = b - a;
        let s: String = label.chars().take(width).collect();
        let pad = width - s.chars().count();
        let l = pad / 2;
        out.push_str(&" ".repeat(l));
        out.push_str(&s);
        out.push_str(&" ".repeat(pad - l));
    }
    out
}

/// Which of `n` HUD buttons a click at column `col` lands on.
pub fn hud_hit(col: usize, w: usize, n: usize) -> usize {
    zones(w, n)
        .iter()
        .position(|&(_, b)| col < b)
        .unwrap_or(n.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_2x2() -> Canvas {
        let mut c = Canvas::new(2, 2, false);
        c.lum = vec![1.0, -1.0, 0.0, 0.5];
        c.base = vec![[1.0, 0.0, 0.0]; 4];
        c
    }

    #[test]
    fn ascii_maps_brightness_and_empties() {
        let rows = canvas_2x2().encode(Mode::Ascii, Palette::Natural);
        assert_eq!(rows, vec!["@ ".to_string(), format!(" {}", Canvas::glyph(0.5))]);
        assert!(rows.iter().all(|r| !r.contains('\x1b')));
    }

    #[test]
    fn blocks_use_half_glyphs() {
        let mut c = Canvas::new(3, 2, false);
        c.lum = vec![1.0, 1.0, -1.0, 1.0, -1.0, 1.0];
        c.base = vec![[1.0, 1.0, 1.0]; 6];
        let rows = c.encode(Mode::Blocks, Palette::Natural);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(r.contains('\u{2580}')); // both pixels set (top fg, bottom bg)
        assert!(r.contains('\u{2584}')); // bottom only
        assert!(r.ends_with("\x1b[0m"));
    }

    #[test]
    fn colour_escapes_are_deduplicated() {
        let mut c = Canvas::new(4, 1, false);
        c.lum = vec![1.0; 4];
        c.base = vec![[0.0, 1.0, 0.0]; 4];
        let row = &c.encode(Mode::AsciiColor, Palette::Natural)[0];
        assert_eq!(row.matches("38;2;0;255;0m").count(), 1);
    }

    #[test]
    fn palette_tints_by_luma() {
        let white = Palette::Green.apply([0.85, 0.85, 0.85]);
        assert!(white[1] > white[0]);
        assert_eq!(Palette::Natural.apply([0.1, 0.2, 0.3]), [0.1, 0.2, 0.3]);
    }

    #[test]
    fn modes_and_palettes_cycle() {
        let mut m = Mode::Ascii;
        for _ in 0..3 {
            m = m.next();
        }
        assert_eq!(m, Mode::Ascii);
        let mut p = Palette::Natural;
        for _ in 0..4 {
            p = p.next();
        }
        assert_eq!(p, Palette::Natural);
        assert_eq!(Mode::parse("Blocks"), Some(Mode::Blocks));
        assert_eq!(Palette::parse("nope"), None);
    }

    #[test]
    fn hud_width_and_hits_agree() {
        for w in [7, 40, 81] {
            let bar = hud(w, &["[ - ]", "auto", "mode", "[ + ]"]);
            assert_eq!(bar.chars().count(), w);
            assert_eq!(hud_hit(0, w, 4), 0);
            assert_eq!(hud_hit(w - 1, w, 4), 3);
        }
        assert_eq!(hud_hit(10, 40, 4), 1);
        assert_eq!(hud_hit(30, 40, 4), 3);
    }
}

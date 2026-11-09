//! Turning a pixel grid into terminal text: ASCII ramp, coloured ASCII, or
//! half-block colour (truecolor, with 256- and 16-colour fallbacks), plus the touch HUD bar.

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

/// How many colours the terminal can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit RGB (`ESC[38;2;r;g;bm`).
    TrueColor,
    /// The xterm 256-colour palette (`ESC[38;5;nm`).
    Ansi256,
    /// The 16 basic colours (`ESC[30..37m`, `ESC[90..97m`).
    Ansi16,
}

impl ColorDepth {
    pub fn label(self) -> &'static str {
        match self {
            ColorDepth::TrueColor => "truecolor",
            ColorDepth::Ansi256 => "256",
            ColorDepth::Ansi16 => "16",
        }
    }

    /// Parses an explicit setting. `auto` is handled by the caller (it means [`ColorDepth::detect`]).
    pub fn parse(s: &str) -> Option<ColorDepth> {
        match s.to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" | "true" => Some(ColorDepth::TrueColor),
            "256" => Some(ColorDepth::Ansi256),
            "16" => Some(ColorDepth::Ansi16),
            _ => None,
        }
    }

    /// Guesses the terminal's colour depth from the environment.
    pub fn detect() -> ColorDepth {
        Self::detect_from(|k| std::env::var(k).ok())
    }

    /// [`ColorDepth::detect`] with the environment passed in, for testing.
    pub fn detect_from(get: impl Fn(&str) -> Option<String>) -> ColorDepth {
        let lower = |k: &str| get(k).map(|v| v.to_ascii_lowercase());
        if matches!(lower("COLORTERM").as_deref(), Some("truecolor" | "24bit")) {
            return ColorDepth::TrueColor;
        }
        let term = lower("TERM");
        let term_program = get("TERM_PROGRAM");
        let known_truecolor_program = matches!(
            term_program.as_deref(),
            Some("iTerm.app" | "vscode" | "WezTerm" | "ghostty" | "Hyper")
        );
        let truecolor_term = term.as_deref().is_some_and(|t| {
            ["truecolor", "direct", "kitty", "alacritty"]
                .iter()
                .any(|k| t.contains(k))
        });
        if get("WT_SESSION").is_some() || known_truecolor_program || truecolor_term {
            return ColorDepth::TrueColor;
        }
        match term.as_deref() {
            // No TERM at all (pipes, CI): keep the historical truecolor output.
            None | Some("") => ColorDepth::TrueColor,
            Some(t) if t.contains("256color") => ColorDepth::Ansi256,
            Some(_) => ColorDepth::Ansi16,
        }
    }

    fn paint(self, c: [u8; 3]) -> Paint {
        match self {
            ColorDepth::TrueColor => Paint::Rgb(c),
            ColorDepth::Ansi256 => Paint::Idx(rgb_to_256(c)),
            ColorDepth::Ansi16 => Paint::Basic(rgb_to_16(c)),
        }
    }
}

/// A colour already reduced to what the terminal can show. Comparing these (rather
/// than the original RGB) lets neighbouring cells that quantise to the same colour
/// share one escape sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Paint {
    Rgb([u8; 3]),
    Idx(u8),
    Basic(u8),
}

impl Paint {
    fn write(self, s: &mut String, background: bool) {
        use std::fmt::Write;
        let _ = match self {
            Paint::Rgb(c) => write!(
                s,
                "\x1b[{};2;{};{};{}m",
                if background { 48 } else { 38 },
                c[0],
                c[1],
                c[2]
            ),
            Paint::Idx(i) => write!(s, "\x1b[{};5;{}m", if background { 48 } else { 38 }, i),
            Paint::Basic(i) => {
                let base = if i < 8 { 30 } else { 82 }; // 90 + (i - 8)
                write!(s, "\x1b[{}m", base + i as u32 + if background { 10 } else { 0 })
            }
        };
    }
}

const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn dist2(a: [u8; 3], b: [u8; 3]) -> i32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2)).sum()
}

/// Nearest colour in the xterm 256 palette: the 6x6x6 cube (16..231) or the grey ramp (232..255).
pub fn rgb_to_256(c: [u8; 3]) -> u8 {
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (CUBE_LEVELS[i] as i32 - v as i32).abs())
            .unwrap_or(0)
    };
    let (r, g, b) = (nearest(c[0]), nearest(c[1]), nearest(c[2]));
    let cube = [CUBE_LEVELS[r], CUBE_LEVELS[g], CUBE_LEVELS[b]];
    let avg = (c[0] as i32 + c[1] as i32 + c[2] as i32) / 3;
    let gi = ((avg - 8 + 5) / 10).clamp(0, 23);
    let gv = (8 + 10 * gi) as u8;
    if dist2(c, [gv; 3]) < dist2(c, cube) {
        232 + gi as u8
    } else {
        (16 + 36 * r + 6 * g + b) as u8
    }
}

/// Nearest of the 16 basic colours.
pub fn rgb_to_16(c: [u8; 3]) -> u8 {
    let max = c[0].max(c[1]).max(c[2]) as i32;
    let min = c[0].min(c[1]).min(c[2]) as i32;
    // Greys: nearest of black, dark grey, light grey and white.
    if (max - min) * 4 < max {
        let avg = (c[0] as i32 + c[1] as i32 + c[2] as i32) / 3;
        return [(0, 0u8), (127, 8), (229, 7), (255, 15)]
            .into_iter()
            .min_by_key(|&(v, _)| (v - avg).abs())
            .map_or(0, |(_, i)| i);
    }
    // Plain nearest-colour matching sends every dim tint to grey, which loses the
    // colour entirely. Instead pick the hue from the colour's direction and use its
    // brightness to choose between black, dark grey, the hue, and its bright form.
    if max < 45 {
        return 0;
    }
    if max < 100 {
        return 8;
    }
    let n = |v: u8| v as i32 * 255 / max;
    let dir = [n(c[0]), n(c[1]), n(c[2])];
    // (rgb direction, standard colour number): red, yellow, green, cyan, blue, magenta.
    let hue = [
        ([255, 0, 0], 1u8),
        ([255, 255, 0], 3),
        ([0, 255, 0], 2),
        ([0, 255, 255], 6),
        ([0, 0, 255], 4),
        ([255, 0, 255], 5),
    ]
    .into_iter()
    .min_by_key(|(d, _)| (0..3).map(|i| (d[i] - dir[i]).pow(2)).sum::<i32>())
    .map_or(1, |(_, i)| i);
    if max >= 230 {
        hue + 8
    } else {
        hue
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
        self.encode_with(mode, palette, ColorDepth::TrueColor)
    }

    /// Like [`Canvas::encode`], reducing colours to `depth`.
    pub fn encode_with(&self, mode: Mode, palette: Palette, depth: ColorDepth) -> Vec<String> {
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
                    let mut row = RowWriter::new(depth);
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
                    let mut row = RowWriter::new(depth);
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
struct RowWriter {
    s: String,
    depth: ColorDepth,
    fg: Option<Paint>,
    bg: Option<Paint>,
}

impl RowWriter {
    fn new(depth: ColorDepth) -> Self {
        RowWriter {
            s: String::new(),
            depth,
            fg: None,
            bg: None,
        }
    }

    fn cell(&mut self, ch: char, fg: Option<[u8; 3]>, bg: Option<[u8; 3]>) {
        let fg = fg.map(|c| self.depth.paint(c));
        let bg = bg.map(|c| self.depth.paint(c));
        if bg != self.bg {
            match bg {
                Some(p) => p.write(&mut self.s, true),
                None => self.s.push_str("\x1b[49m"),
            }
            self.bg = bg;
        }
        // A space shows no foreground, so don't spend bytes changing it.
        if ch != ' ' && fg != self.fg {
            if let Some(p) = fg {
                p.write(&mut self.s, false);
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

    #[test]
    fn rgb_to_256_hits_known_entries() {
        assert_eq!(rgb_to_256([0, 0, 0]), 16);
        assert_eq!(rgb_to_256([255, 255, 255]), 231);
        assert_eq!(rgb_to_256([255, 0, 0]), 196);
        assert_eq!(rgb_to_256([95, 135, 175]), 67);
        // Mid grey is closer to the grey ramp than to any cube entry.
        assert_eq!(rgb_to_256([128, 128, 128]), 244);
    }

    #[test]
    fn rgb_to_16_picks_the_nearest_basic_colour() {
        assert_eq!(rgb_to_16([0, 0, 0]), 0);
        assert_eq!(rgb_to_16([255, 255, 255]), 15);
        assert_eq!(rgb_to_16([255, 0, 0]), 9);
        assert_eq!(rgb_to_16([200, 0, 0]), 1);
        assert_eq!(rgb_to_16([0, 250, 10]), 10);
    }

    #[test]
    fn rgb_to_16_keeps_the_hue_of_dim_colours() {
        // Shaded blue/cyan must stay cyan-ish, not collapse to grey.
        assert_eq!(rgb_to_16([50, 120, 160]), 6); // mid cyan-blue
        assert_eq!(rgb_to_16([120, 210, 255]), 14); // bright ice blue -> bright cyan
        assert_eq!(rgb_to_16([200, 110, 40]), 3); // orange -> yellow family
                                                  // Very dark colours fade out, with one step of dark grey before black.
        assert_eq!(rgb_to_16([20, 30, 40]), 0);
        assert_eq!(rgb_to_16([40, 60, 90]), 8);
        // Greys stay grey.
        assert_eq!(rgb_to_16([130, 130, 130]), 8);
        assert_eq!(rgb_to_16([225, 225, 225]), 7);
    }

    fn two_colour_row(depth: ColorDepth, mode: Mode) -> String {
        let mut c = Canvas::new(2, 2, false);
        c.lum = vec![1.0; 4];
        c.base = vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        c.encode_with(mode, Palette::Natural, depth).remove(0)
    }

    #[test]
    fn depth_changes_the_escape_format() {
        let t = two_colour_row(ColorDepth::TrueColor, Mode::Blocks);
        assert!(t.contains("38;2;255;0;0m") && t.contains("48;2;0;255;0m"));

        let p = two_colour_row(ColorDepth::Ansi256, Mode::Blocks);
        assert!(p.contains("38;5;196m") && p.contains("48;5;46m"), "{p:?}");
        assert!(!p.contains(";2;"));

        let b = two_colour_row(ColorDepth::Ansi16, Mode::Blocks);
        assert!(b.contains("\x1b[91m"), "bright red fg: {b:?}"); // 90 + 1
        assert!(b.contains("\x1b[102m"), "bright green bg: {b:?}"); // 100 + 2
        assert!(!b.contains(";2;") && !b.contains(";5;"));
        assert!(b.ends_with("\x1b[0m"));
    }

    #[test]
    fn basic_colours_use_the_low_codes() {
        let mut p = String::new();
        Paint::Basic(1).write(&mut p, false);
        Paint::Basic(1).write(&mut p, true);
        Paint::Basic(9).write(&mut p, false);
        Paint::Basic(9).write(&mut p, true);
        assert_eq!(p, "\x1b[31m\x1b[41m\x1b[91m\x1b[101m");
    }

    #[test]
    fn nearby_colours_share_one_escape_at_low_depth() {
        let mut c = Canvas::new(4, 1, false);
        c.lum = vec![1.0; 4];
        c.base = vec![
            [0.0, 1.0, 0.0],
            [0.05, 0.95, 0.02],
            [0.0, 0.97, 0.0],
            [0.02, 1.0, 0.04],
        ];
        let row = &c.encode_with(Mode::AsciiColor, Palette::Natural, ColorDepth::Ansi16)[0];
        assert_eq!(
            row.matches("\x1b[").count(),
            2,
            "one colour set + one reset: {row:?}"
        );
    }

    #[test]
    fn plain_ascii_ignores_depth() {
        let c = canvas_2x2();
        for d in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            assert_eq!(
                c.encode_with(Mode::Ascii, Palette::Natural, d),
                c.encode(Mode::Ascii, Palette::Natural)
            );
        }
    }

    fn detect(vars: &[(&str, &str)]) -> ColorDepth {
        ColorDepth::detect_from(|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn detects_depth_from_the_environment() {
        assert_eq!(
            detect(&[("COLORTERM", "truecolor"), ("TERM", "xterm")]),
            ColorDepth::TrueColor
        );
        assert_eq!(detect(&[("COLORTERM", "24bit")]), ColorDepth::TrueColor);
        assert_eq!(
            detect(&[("WT_SESSION", "x"), ("TERM", "xterm")]),
            ColorDepth::TrueColor
        );
        assert_eq!(
            detect(&[("TERM_PROGRAM", "iTerm.app"), ("TERM", "xterm")]),
            ColorDepth::TrueColor
        );
        assert_eq!(detect(&[("TERM", "xterm-kitty")]), ColorDepth::TrueColor);
        assert_eq!(detect(&[("TERM", "xterm-256color")]), ColorDepth::Ansi256);
        assert_eq!(detect(&[("TERM", "screen-256color")]), ColorDepth::Ansi256);
        assert_eq!(detect(&[("TERM", "xterm")]), ColorDepth::Ansi16);
        assert_eq!(detect(&[("TERM", "linux")]), ColorDepth::Ansi16);
        // Nothing known: keep the old behaviour.
        assert_eq!(detect(&[]), ColorDepth::TrueColor);
    }

    #[test]
    fn depth_parses_and_labels() {
        for d in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            assert_eq!(ColorDepth::parse(d.label()), Some(d));
        }
        assert_eq!(ColorDepth::parse("24bit"), Some(ColorDepth::TrueColor));
        assert_eq!(ColorDepth::parse("8"), None);
    }
}

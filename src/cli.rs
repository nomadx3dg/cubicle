//! Command-line parsing (hand-rolled to keep the dependency list tiny).

use crate::display::{ColorDepth, Mode, Palette};
use crate::mesh::DEFAULT_CREASE_DEG;
use std::path::PathBuf;

pub const USAGE: &str = "\
cubicle - interactive 3D renderer for your terminal

USAGE:
    cubicle [OPTIONS] [FILE]

FILE:
    model.obj      view a Wavefront OBJ model
    image.png      view a PNG as ASCII art
    (none)         view the built-in spinning cube

OPTIONS:
    -m, --mode <ascii|color|blocks>      render mode (default: ascii)
    -p, --palette <natural|green|amber|ice>
                                         colour palette for color/blocks (default: natural)
        --colors <auto|truecolor|256|16>  colour depth to emit (default: auto, from COLORTERM/TERM)
        --texture <FILE.png>             texture-map a PNG onto the model (uses the model's vt coordinates;
                                         the built-in cube has them too)
        --flat                           start with flat shading (models)
        --two-sided                      draw back faces too (models)
        --crease <DEG>                   smooth-shading crease angle (default: 60)
        --no-auto                        start with auto-spin off (models)
        --invert                         invert brightness for light terminals (images)
        --once                           render one frame to stdout and exit (no TTY needed)
        --size <WxH>                     frame size for --once (default: terminal size or 80x24)
        --rot <X,Y>                      start / --once rotation in degrees (models)
    -h, --help                           print this help
    -V, --version                        print the version

KEYS (interactive):
    arrows turn   +/- zoom   space auto-spin   m mode   c palette
    s smooth/flat   b two-sided   t texture on/off   i invert (images)   r reset view   q/Esc quit
";

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Cube,
    Obj(PathBuf),
    Png(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Config {
    pub input: Input,
    pub mode: Mode,
    pub palette: Palette,
    /// Colour depth for the colour modes. Resolved from the environment unless `--colors` is given.
    pub depth: ColorDepth,
    /// PNG to map onto the model, if any.
    pub texture: Option<PathBuf>,
    pub smooth: bool,
    pub two_sided: bool,
    pub auto: bool,
    pub invert: bool,
    pub once: bool,
    pub size: Option<(usize, usize)>,
    /// (x, y) rotation in radians.
    pub rot: Option<(f32, f32)>,
    pub crease: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            input: Input::Cube,
            mode: Mode::Ascii,
            palette: Palette::Natural,
            depth: ColorDepth::TrueColor,
            texture: None,
            smooth: true,
            two_sided: false,
            auto: true,
            invert: false,
            once: false,
            size: None,
            rot: None,
            crease: DEFAULT_CREASE_DEG,
        }
    }
}

#[derive(Debug)]
pub enum Parsed {
    Run(Box<Config>),
    Help,
    Version,
}

fn value<I: Iterator<Item = String>>(it: &mut I, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn num(s: &str, flag: &str) -> Result<f32, String> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|f| f.is_finite())
        .ok_or_else(|| format!("{flag}: '{s}' is not a number"))
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cfg = Config::default();
    let mut file: Option<String> = None;
    let mut depth: Option<ColorDepth> = None; // None = auto-detect
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        // Allow --flag=value as well as --flag value.
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let take = |it: &mut <I as IntoIterator>::IntoIter| match inline.clone() {
            Some(v) => Ok(v),
            None => value(it, &flag),
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "-m" | "--mode" => {
                let v = take(&mut it)?;
                cfg.mode =
                    Mode::parse(&v).ok_or_else(|| format!("unknown mode '{v}' (ascii, color, blocks)"))?;
            }
            "-p" | "--palette" => {
                let v = take(&mut it)?;
                cfg.palette = Palette::parse(&v)
                    .ok_or_else(|| format!("unknown palette '{v}' (natural, green, amber, ice)"))?;
            }
            "--colors" | "--colours" => {
                let v = take(&mut it)?;
                depth =
                    if v.eq_ignore_ascii_case("auto") {
                        None
                    } else {
                        Some(ColorDepth::parse(&v).ok_or_else(|| {
                            format!("unknown colour depth '{v}' (auto, truecolor, 256, 16)")
                        })?)
                    };
            }
            "--texture" => cfg.texture = Some(take(&mut it)?.into()),
            "--flat" => cfg.smooth = false,
            "--two-sided" => cfg.two_sided = true,
            "--no-auto" => cfg.auto = false,
            "--invert" => cfg.invert = true,
            "--once" => cfg.once = true,
            "--crease" => {
                let v = take(&mut it)?;
                let d = num(&v, "--crease")?;
                if !(0.0..=180.0).contains(&d) {
                    return Err("--crease must be between 0 and 180".into());
                }
                cfg.crease = d;
            }
            "--size" => {
                let v = take(&mut it)?;
                let (w, h) = v
                    .split_once(['x', 'X'])
                    .and_then(|(w, h)| Some((w.parse::<usize>().ok()?, h.parse::<usize>().ok()?)))
                    .filter(|&(w, h)| w > 0 && h > 0)
                    .ok_or_else(|| format!("--size: '{v}' is not WxH (e.g. 80x24)"))?;
                cfg.size = Some((w, h));
            }
            "--rot" => {
                let v = take(&mut it)?;
                let (x, y) = v
                    .split_once(',')
                    .ok_or_else(|| format!("--rot: '{v}' is not X,Y (degrees)"))?;
                cfg.rot = Some((num(x, "--rot")?.to_radians(), num(y, "--rot")?.to_radians()));
            }
            f if f.starts_with('-') && f.len() > 1 => {
                return Err(format!("unknown option '{f}' (try --help)"))
            }
            _ => {
                if file.replace(a).is_some() {
                    return Err("only one FILE may be given".into());
                }
            }
        }
    }
    if let Some(f) = file {
        let ext = std::path::Path::new(&f)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        cfg.input = match ext.as_deref() {
            Some("obj") => Input::Obj(f.into()),
            Some("png") => Input::Png(f.into()),
            _ => return Err(format!("'{f}': unsupported file type (expected .obj or .png)")),
        };
    }
    cfg.depth = depth.unwrap_or_else(ColorDepth::detect);
    if cfg.texture.is_some() && matches!(cfg.input, Input::Png(_)) {
        return Err("--texture applies to 3D models, not to a PNG being viewed".into());
    }
    Ok(Parsed::Run(Box::new(cfg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }
    fn cfg(args: &[&str]) -> Config {
        match parse(args).unwrap() {
            Parsed::Run(c) => *c,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn defaults_run_the_cube() {
        let c = cfg(&[]);
        assert_eq!(c.input, Input::Cube);
        assert_eq!(c.mode, Mode::Ascii);
        assert!(c.smooth && c.auto && !c.once);
    }

    #[test]
    fn file_type_is_detected_by_extension() {
        assert_eq!(cfg(&["a/b/Model.OBJ"]).input, Input::Obj("a/b/Model.OBJ".into()));
        assert_eq!(cfg(&["pic.png"]).input, Input::Png("pic.png".into()));
        assert!(parse(&["thing.stl"]).is_err());
        assert!(parse(&["a.obj", "b.obj"]).is_err());
    }

    #[test]
    fn options_in_both_styles() {
        let c = cfg(&[
            "-m",
            "blocks",
            "--palette=amber",
            "--flat",
            "--two-sided",
            "--no-auto",
            "--once",
        ]);
        assert_eq!(c.mode, Mode::Blocks);
        assert_eq!(c.palette, Palette::Amber);
        assert!(!c.smooth && c.two_sided && !c.auto && c.once);
    }

    #[test]
    fn size_rot_crease() {
        let c = cfg(&["--size", "100x30", "--rot", "90,180", "--crease=45"]);
        assert_eq!(c.size, Some((100, 30)));
        let (x, y) = c.rot.unwrap();
        assert!((x - std::f32::consts::FRAC_PI_2).abs() < 1e-5 && (y - std::f32::consts::PI).abs() < 1e-5);
        assert_eq!(c.crease, 45.0);
    }

    #[test]
    fn bad_input_gives_clear_errors() {
        assert!(parse(&["--mode"]).unwrap_err().contains("needs a value"));
        assert!(parse(&["--mode", "x"]).unwrap_err().contains("unknown mode"));
        assert!(parse(&["--size", "0x5"]).is_err());
        assert!(parse(&["--size", "abc"]).is_err());
        assert!(parse(&["--rot", "1"]).is_err());
        assert!(parse(&["--crease", "400"]).is_err());
        assert!(parse(&["--bogus"]).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn colour_depth_option() {
        assert_eq!(cfg(&["--colors", "256"]).depth, ColorDepth::Ansi256);
        assert_eq!(cfg(&["--colors=16"]).depth, ColorDepth::Ansi16);
        assert_eq!(cfg(&["--colours", "truecolor"]).depth, ColorDepth::TrueColor);
        // `auto` (and no flag at all) resolve from the environment.
        assert_eq!(cfg(&["--colors", "auto"]).depth, ColorDepth::detect());
        assert_eq!(cfg(&[]).depth, ColorDepth::detect());
        assert!(parse(&["--colors", "8"])
            .unwrap_err()
            .contains("unknown colour depth"));
        assert!(parse(&["--colors"]).unwrap_err().contains("needs a value"));
    }

    #[test]
    fn texture_option() {
        let c = cfg(&["--texture", "t.png", "m.obj"]);
        assert_eq!(c.texture, Some("t.png".into()));
        assert_eq!(cfg(&["--texture=t.png"]).texture, Some("t.png".into()));
        assert!(parse(&["--texture"]).is_err());
        assert!(parse(&["--texture", "t.png", "pic.png"]).is_err());
    }

    #[test]
    fn help_and_version() {
        assert!(matches!(parse(&["-h"]).unwrap(), Parsed::Help));
        assert!(matches!(parse(&["--version"]).unwrap(), Parsed::Version));
    }
}

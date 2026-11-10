//! User settings shared by the menu and the viewers, with a tiny `key=value`
//! file format (no serde) so they persist between runs.

use crate::cli::Config;
use crate::display::{ColorDepth, Mode, Palette};
use std::path::{Path, PathBuf};

/// Which processor rasterises the 3D view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The built-in CPU rasteriser (always available).
    Cpu,
    /// A GPU render pipeline via `wgpu` (needs a build with `--features gpu` and a Vulkan/Metal/DX12 adapter).
    Gpu,
    /// Both at once: the GPU draws the top half of the frame while the CPU draws the bottom half.
    Both,
}

impl Backend {
    pub fn next(self) -> Backend {
        match self {
            Backend::Cpu => Backend::Gpu,
            Backend::Gpu => Backend::Both,
            Backend::Both => Backend::Cpu,
        }
    }
    pub fn prev(self) -> Backend {
        self.next().next()
    }
    pub fn label(self) -> &'static str {
        match self {
            Backend::Cpu => "cpu",
            Backend::Gpu => "gpu",
            Backend::Both => "cpu+gpu",
        }
    }
    pub fn parse(s: &str) -> Option<Backend> {
        match s.to_ascii_lowercase().as_str() {
            "cpu" => Some(Backend::Cpu),
            "gpu" => Some(Backend::Gpu),
            "both" | "cpu+gpu" | "gpu+cpu" | "hybrid" => Some(Backend::Both),
            _ => None,
        }
    }
    pub fn uses_gpu(self) -> bool {
        self != Backend::Cpu
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub backend: Backend,
    pub mode: Mode,
    pub palette: Palette,
    /// `None` means auto-detect from the environment.
    pub depth: Option<ColorDepth>,
    pub smooth: bool,
    pub two_sided: bool,
    pub auto: bool,
    pub texture: Option<PathBuf>,
    /// Where the file browser starts.
    pub browse_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            backend: Backend::Cpu,
            mode: Mode::Ascii,
            palette: Palette::Natural,
            depth: None,
            smooth: true,
            two_sided: false,
            auto: true,
            texture: None,
            browse_dir: None,
        }
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// Settings equivalent to command-line flags.
    pub fn from_config(cfg: &Config) -> Settings {
        Settings {
            backend: cfg.backend,
            mode: cfg.mode,
            palette: cfg.palette,
            depth: Some(cfg.depth),
            smooth: cfg.smooth,
            two_sided: cfg.two_sided,
            auto: cfg.auto,
            texture: cfg.texture.clone(),
            browse_dir: None,
        }
    }

    /// Copies these settings into a viewer configuration.
    pub fn apply(&self, cfg: &mut Config) {
        cfg.backend = self.backend;
        cfg.mode = self.mode;
        cfg.palette = self.palette;
        cfg.depth = self.depth.unwrap_or_else(ColorDepth::detect);
        cfg.smooth = self.smooth;
        cfg.two_sided = self.two_sided;
        cfg.auto = self.auto;
        cfg.texture = self.texture.clone();
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# cubicle settings (edited by the menu; safe to edit by hand)\n");
        s += &format!(
            "backend={}\n",
            match self.backend {
                Backend::Both => "both",
                b => b.label(),
            }
        );
        s += &format!("mode={}\n", self.mode.label());
        s += &format!("palette={}\n", self.palette.label());
        s += &format!("colors={}\n", self.depth.map_or("auto", |d| d.label()));
        s += &format!("smooth={}\n", self.smooth);
        s += &format!("two_sided={}\n", self.two_sided);
        s += &format!("auto_spin={}\n", self.auto);
        if let Some(t) = &self.texture {
            s += &format!("texture={}\n", t.to_string_lossy());
        }
        if let Some(d) = &self.browse_dir {
            s += &format!("browse_dir={}\n", d.to_string_lossy());
        }
        s
    }

    /// Lenient parser: unknown keys and bad values are ignored (defaults stay).
    pub fn from_text(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "backend" => s.backend = Backend::parse(v).unwrap_or(s.backend),
                "mode" => s.mode = Mode::parse(v).unwrap_or(s.mode),
                "palette" => s.palette = Palette::parse(v).unwrap_or(s.palette),
                "colors" => {
                    if v == "auto" {
                        s.depth = None;
                    } else if let Some(d) = ColorDepth::parse(v) {
                        s.depth = Some(d);
                    }
                }
                "smooth" => s.smooth = parse_bool(v).unwrap_or(s.smooth),
                "two_sided" => s.two_sided = parse_bool(v).unwrap_or(s.two_sided),
                "auto_spin" => s.auto = parse_bool(v).unwrap_or(s.auto),
                "texture" if !v.is_empty() => s.texture = Some(v.into()),
                "browse_dir" if !v.is_empty() => s.browse_dir = Some(v.into()),
                _ => {}
            }
        }
        s
    }

    /// `$CUBICLE_CONFIG`, else `$XDG_CONFIG_HOME/cubicle/settings.conf`, else
    /// `~/.config/cubicle/settings.conf`.
    pub fn config_path() -> Option<PathBuf> {
        Self::config_path_from(|k| std::env::var(k).ok())
    }

    pub fn config_path_from(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
        if let Some(p) = get("CUBICLE_CONFIG").filter(|p| !p.is_empty()) {
            return Some(p.into());
        }
        let base = get("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                get("HOME")
                    .filter(|p| !p.is_empty())
                    .map(|h| Path::new(&h).join(".config"))
            })?;
        Some(base.join("cubicle").join("settings.conf"))
    }

    /// Loads the saved settings, or defaults if there is no (readable) file.
    pub fn load() -> Settings {
        Self::config_path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .map(|t| Settings::from_text(&t))
            .unwrap_or_default()
    }

    /// Saves to the default location. Failure (read-only home, no HOME) is
    /// reported to the caller, never fatal.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory"))?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_cycles_and_parses() {
        assert_eq!(Backend::Cpu.next(), Backend::Gpu);
        assert_eq!(Backend::Cpu.prev(), Backend::Both);
        assert_eq!(Backend::Both.next(), Backend::Cpu);
        assert_eq!(Backend::parse("CPU+GPU"), Some(Backend::Both));
        assert_eq!(Backend::parse("tpu"), None);
        assert!(!Backend::Cpu.uses_gpu() && Backend::Both.uses_gpu());
    }

    #[test]
    fn text_round_trip() {
        let s = Settings {
            backend: Backend::Both,
            mode: Mode::Blocks,
            palette: Palette::Amber,
            depth: Some(ColorDepth::Ansi256),
            smooth: false,
            two_sided: true,
            auto: false,
            texture: Some("/tmp/a b/t.png".into()),
            browse_dir: Some("/home/me/models".into()),
        };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        let d = Settings::default();
        assert_eq!(Settings::from_text(&d.to_text()), d);
    }

    #[test]
    fn parser_is_lenient() {
        let s = Settings::from_text(
            "garbage\nmode=blocks\nbackend=quantum\nsmooth=maybe\n# c\nunknown=1\ncolors=auto\n",
        );
        assert_eq!(s.mode, Mode::Blocks);
        assert_eq!(s.backend, Backend::Cpu);
        assert!(s.smooth);
        assert_eq!(s.depth, None);
    }

    #[test]
    fn config_path_resolution() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        let p = Settings::config_path_from(env(&[("CUBICLE_CONFIG", "/x/y.conf"), ("HOME", "/h")]));
        assert_eq!(p, Some("/x/y.conf".into()));
        let p = Settings::config_path_from(env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")]));
        assert_eq!(p, Some("/xdg/cubicle/settings.conf".into()));
        let p = Settings::config_path_from(env(&[("HOME", "/h")]));
        assert_eq!(p, Some("/h/.config/cubicle/settings.conf".into()));
        assert_eq!(Settings::config_path_from(env(&[])), None);
    }

    #[test]
    fn save_and_load_through_a_file() {
        let dir = std::env::temp_dir().join(format!("cubicle-settings-{}", std::process::id()));
        let path = dir.join("nested").join("s.conf");
        let s = Settings {
            mode: Mode::AsciiColor,
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        // A missing file just gives defaults.
        assert_eq!(Settings::load_from(&dir.join("absent.conf")), Settings::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}

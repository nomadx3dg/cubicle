//! The main menu, render-settings page and file-browser pages.
//!
//! Like the viewers, this is a state machine: `on_key` / `on_mouse` mutate it
//! and return an [`Action`]; `draw` returns the rows to show. The terminal
//! loop lives in `app.rs`, so everything here is testable without a TTY.
//! Settings are saved after every change.

use crate::browser::{fit, Browser, Pick};
use crate::display::{ColorDepth, Mode, Palette};
use crate::gpu::GpuInfo;
use crate::settings::{Backend, Settings};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use std::path::{Path, PathBuf};

/// What the main loop should do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Launch(Launch),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    Cube,
    Model(PathBuf),
    Image(PathBuf),
}

/// What the GPU probe found (probed when the settings page opens).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuStatus {
    Unknown,
    Ready(GpuInfo),
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowseFor {
    Model,
    Image,
    Texture,
}

enum Page {
    Main,
    Settings,
    Browse(BrowseFor),
}

const MAIN_ITEMS: [&str; 5] = [
    "Spin the cube",
    "Browse models (.obj)…",
    "View an image (.png)…",
    "Render settings…",
    "Quit",
];
const SETTING_COUNT: usize = 9;
const ROW_TEXTURE: usize = 7;
const ROW_BACK: usize = 8;
/// Rows above the list (blank, title, rule) and below it (context, help).
const HEADER: usize = 3;
const FOOTER: usize = 2;

pub struct Menu {
    pub settings: Settings,
    page: Page,
    sel: usize,
    top: usize,
    browser: Option<Browser>,
    pub gpu: GpuStatus,
    /// A one-line error or note shown until the next key press.
    pub message: Option<String>,
    save_path: Option<PathBuf>,
    probe: fn() -> Result<GpuInfo, String>,
}

fn cycle<T: Copy + PartialEq>(options: &[T], cur: T, dir: isize) -> T {
    let i = options.iter().position(|&o| o == cur).unwrap_or(0) as isize;
    options[(i + dir).rem_euclid(options.len() as isize) as usize]
}

impl Menu {
    pub fn new(settings: Settings, save_path: Option<PathBuf>) -> Menu {
        Menu::with_probe(settings, save_path, crate::gpu::probe)
    }

    pub fn with_probe(
        settings: Settings,
        save_path: Option<PathBuf>,
        probe: fn() -> Result<GpuInfo, String>,
    ) -> Menu {
        Menu {
            settings,
            page: Page::Main,
            sel: 0,
            top: 0,
            browser: None,
            gpu: GpuStatus::Unknown,
            message: None,
            save_path,
            probe,
        }
    }

    pub fn show_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
    }

    fn persist(&mut self) {
        if let Some(p) = &self.save_path {
            if let Err(e) = self.settings.save_to(p) {
                self.message = Some(format!("could not save settings: {e}"));
            }
        }
    }

    fn goto(&mut self, page: Page, sel: usize) {
        self.page = page;
        self.sel = sel;
        self.top = 0;
    }

    fn probe_gpu(&mut self) {
        if self.gpu == GpuStatus::Unknown {
            self.gpu = match (self.probe)() {
                Ok(i) => GpuStatus::Ready(i),
                Err(e) => GpuStatus::Unavailable(e),
            };
        }
    }

    fn start_dir(&self, what: BrowseFor) -> PathBuf {
        if let Some(d) = self.settings.browse_dir.as_ref().filter(|d| d.is_dir()) {
            return d.clone();
        }
        let sample = if what == BrowseFor::Model {
            "models"
        } else {
            "examples"
        };
        if Path::new(sample).is_dir() {
            sample.into()
        } else {
            ".".into()
        }
    }

    fn open_browser(&mut self, what: BrowseFor) {
        let ext: &[&'static str] = if what == BrowseFor::Model {
            &["obj"]
        } else {
            &["png"]
        };
        let start = if what == BrowseFor::Texture {
            self.settings
                .texture
                .as_deref()
                .and_then(Path::parent)
                .filter(|p| p.is_dir())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.start_dir(what))
        } else {
            self.start_dir(what)
        };
        self.browser = Some(Browser::new(&start, ext));
        self.page = Page::Browse(what);
    }

    fn close_browser(&mut self, what: BrowseFor) {
        self.browser = None;
        if what == BrowseFor::Texture {
            self.goto(Page::Settings, ROW_TEXTURE);
        } else {
            self.goto(Page::Main, if what == BrowseFor::Model { 1 } else { 2 });
        }
    }

    // ---------------------------------------------------------------- settings

    /// Changes setting `i` by one step in direction `dir` (+1 / -1).
    fn change(&mut self, i: usize, dir: isize) {
        let s = &mut self.settings;
        match i {
            0 => {
                s.backend = if dir >= 0 {
                    s.backend.next()
                } else {
                    s.backend.prev()
                }
            }
            1 => s.mode = cycle(&[Mode::Ascii, Mode::AsciiColor, Mode::Blocks], s.mode, dir),
            2 => {
                s.palette = cycle(
                    &[Palette::Natural, Palette::Green, Palette::Amber, Palette::Ice],
                    s.palette,
                    dir,
                )
            }
            3 => {
                let opts = [
                    None,
                    Some(ColorDepth::TrueColor),
                    Some(ColorDepth::Ansi256),
                    Some(ColorDepth::Ansi16),
                ];
                s.depth = cycle(&opts, s.depth, dir);
            }
            4 => s.smooth = !s.smooth,
            5 => s.two_sided = !s.two_sided,
            6 => s.auto = !s.auto,
            ROW_TEXTURE => {
                self.open_browser(BrowseFor::Texture);
                return;
            }
            _ => return,
        }
        self.persist();
    }

    fn setting_line(&self, i: usize) -> (&'static str, String) {
        let s = &self.settings;
        match i {
            0 => {
                let na = matches!(self.gpu, GpuStatus::Unavailable(_)) && s.backend.uses_gpu();
                (
                    "Backend",
                    format!("{}{}", s.backend.label(), if na { " (no gpu)" } else { "" }),
                )
            }
            1 => ("Render mode", s.mode.label().into()),
            2 => ("Palette", s.palette.label().into()),
            3 => ("Colours", s.depth.map_or("auto", |d| d.label()).into()),
            4 => ("Shading", if s.smooth { "smooth" } else { "flat" }.into()),
            5 => ("Faces", if s.two_sided { "two-sided" } else { "culled" }.into()),
            6 => ("Auto-spin", if s.auto { "on" } else { "off" }.into()),
            ROW_TEXTURE => (
                "Texture",
                s.texture
                    .as_deref()
                    .and_then(Path::file_name)
                    .map_or("none".into(), |n| n.to_string_lossy().into_owned()),
            ),
            _ => ("‹ Back", String::new()),
        }
    }

    fn setting_help(&self, i: usize) -> String {
        match i {
            0 => {
                let gpu = match &self.gpu {
                    GpuStatus::Ready(info) => format!("GPU: {}", info.summary()),
                    GpuStatus::Unavailable(e) => format!("GPU unavailable: {e}"),
                    GpuStatus::Unknown => "GPU: not checked".into(),
                };
                match self.settings.backend {
                    Backend::Both => format!("{gpu} · GPU draws the top half, CPU the bottom"),
                    _ => gpu,
                }
            }
            1 => "ascii: plain text · color: tinted text · blocks: half-block pixels".into(),
            2 => "natural keeps the model / image colours".into(),
            3 => "auto detects your terminal; pick 256 or 16 for older ones".into(),
            4 => "smooth blends normals; flat shows every facet".into(),
            5 => "two-sided also draws back faces (open meshes)".into(),
            6 => "spin the model by itself".into(),
            ROW_TEXTURE => "Enter: choose a PNG · x: remove. Needs a model with UVs".into(),
            _ => String::new(),
        }
    }

    fn item_count(&self) -> usize {
        match self.page {
            Page::Main => MAIN_ITEMS.len(),
            _ => SETTING_COUNT,
        }
    }

    // ------------------------------------------------------------------- input

    fn activate_main(&mut self, i: usize) -> Action {
        match i {
            0 => Action::Launch(Launch::Cube),
            1 => {
                self.open_browser(BrowseFor::Model);
                Action::None
            }
            2 => {
                self.open_browser(BrowseFor::Image);
                Action::None
            }
            3 => {
                self.probe_gpu();
                self.goto(Page::Settings, 0);
                Action::None
            }
            _ => Action::Quit,
        }
    }

    fn picked(&mut self, what: BrowseFor, path: PathBuf) -> Action {
        if let Some(dir) = self.browser.as_ref().map(|b| b.dir.clone()) {
            if what != BrowseFor::Texture {
                self.settings.browse_dir = Some(dir);
            }
        }
        match what {
            BrowseFor::Texture => {
                self.settings.texture = Some(path);
                self.persist();
                self.close_browser(what);
                Action::None
            }
            BrowseFor::Model => {
                self.persist();
                Action::Launch(Launch::Model(path))
            }
            BrowseFor::Image => {
                self.persist();
                Action::Launch(Launch::Image(path))
            }
        }
    }

    pub fn on_key(&mut self, code: KeyCode) -> Action {
        self.message = None;
        if let Page::Browse(what) = self.page {
            return self.browse_key(what, code);
        }
        let n = self.item_count();
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.sel = (self.sel + 1).min(n - 1),
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.sel = n - 1,
            _ => {}
        }
        match (&self.page, code) {
            (Page::Main, KeyCode::Char('q') | KeyCode::Esc) => return Action::Quit,
            (Page::Main, KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right) => {
                return self.activate_main(self.sel)
            }
            (Page::Main, KeyCode::Char(c @ '1'..='5')) => {
                self.sel = c as usize - '1' as usize;
                return self.activate_main(self.sel);
            }
            (Page::Settings, KeyCode::Esc | KeyCode::Char('q')) => self.goto(Page::Main, 3),
            (Page::Settings, KeyCode::Left | KeyCode::Char('h')) => self.change(self.sel, -1),
            (Page::Settings, KeyCode::Right | KeyCode::Char('l' | ' ')) => self.change(self.sel, 1),
            (Page::Settings, KeyCode::Enter) => {
                if self.sel == ROW_BACK {
                    self.goto(Page::Main, 3);
                } else {
                    self.change(self.sel, 1);
                }
            }
            (Page::Settings, KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x'))
                if self.sel == ROW_TEXTURE =>
            {
                self.settings.texture = None;
                self.persist();
            }
            _ => {}
        }
        Action::None
    }

    fn browse_key(&mut self, what: BrowseFor, code: KeyCode) -> Action {
        let Some(b) = self.browser.as_mut() else {
            self.goto(Page::Main, 0);
            return Action::None;
        };
        let pick = match code {
            KeyCode::Up | KeyCode::Char('k') => {
                b.move_sel(-1);
                Pick::Nothing
            }
            KeyCode::Down | KeyCode::Char('j') => {
                b.move_sel(1);
                Pick::Nothing
            }
            KeyCode::PageUp => {
                b.move_sel(-8);
                Pick::Nothing
            }
            KeyCode::PageDown => {
                b.move_sel(8);
                Pick::Nothing
            }
            KeyCode::Home => {
                b.home();
                Pick::Nothing
            }
            KeyCode::End => {
                b.end();
                Pick::Nothing
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => b.activate(),
            KeyCode::Backspace | KeyCode::Left => {
                b.go_up();
                Pick::Nothing
            }
            KeyCode::Char('h' | '.') => {
                b.toggle_hidden();
                Pick::Nothing
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.close_browser(what);
                return Action::None;
            }
            _ => Pick::Nothing,
        };
        match pick {
            Pick::Chosen(p) => self.picked(what, p),
            _ => Action::None,
        }
    }

    /// Taps / clicks / wheel. `w`,`h` are the terminal size.
    pub fn on_mouse(&mut self, kind: MouseEventKind, col: u16, row: u16, w: u16, h: u16) -> Action {
        let (col, row, w, h) = (col as usize, row as usize, w as usize, h as usize);
        if let Page::Browse(what) = self.page {
            return match kind {
                MouseEventKind::Down(MouseButton::Left) if row == 0 => {
                    self.close_browser(what);
                    Action::None
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    let pick = self.browser.as_mut().map_or(Pick::Nothing, |b| b.tap(row, h));
                    match pick {
                        Pick::Chosen(p) => self.picked(what, p),
                        _ => Action::None,
                    }
                }
                MouseEventKind::ScrollDown => {
                    if let Some(b) = self.browser.as_mut() {
                        b.move_sel(3);
                    }
                    Action::None
                }
                MouseEventKind::ScrollUp => {
                    if let Some(b) = self.browser.as_mut() {
                        b.move_sel(-3);
                    }
                    Action::None
                }
                _ => Action::None,
            };
        }
        self.message = None;
        let n = self.item_count();
        match kind {
            MouseEventKind::ScrollDown => self.sel = (self.sel + 1).min(n - 1),
            MouseEventKind::ScrollUp => self.sel = self.sel.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if matches!(self.page, Page::Settings) && row < HEADER - 1 {
                    self.goto(Page::Main, 3); // tap the title to go back
                    return Action::None;
                }
                let visible = h.saturating_sub(HEADER + FOOTER).max(1);
                if row < HEADER || row >= HEADER + visible {
                    return Action::None;
                }
                let idx = self.top + (row - HEADER);
                if idx >= n {
                    return Action::None;
                }
                self.sel = idx;
                match self.page {
                    Page::Main => return self.activate_main(idx),
                    _ if idx == ROW_BACK => self.goto(Page::Main, 3),
                    _ if idx == ROW_TEXTURE => self.change(idx, 1),
                    // Left third of the row steps back, the rest steps forward.
                    _ => self.change(idx, if col < w / 3 { -1 } else { 1 }),
                }
            }
            _ => {}
        }
        Action::None
    }

    // ----------------------------------------------------------------- drawing

    fn summary(&self) -> String {
        let s = &self.settings;
        format!(
            "{} · {} · {} · {}",
            s.backend.label(),
            s.mode.label(),
            if s.smooth { "smooth" } else { "flat" },
            s.palette.label()
        )
    }

    pub fn draw(&mut self, w: usize, h: usize) -> Vec<String> {
        if let Page::Browse(what) = self.page {
            let title = match what {
                BrowseFor::Model => "‹ Choose a model (.obj)",
                BrowseFor::Image => "‹ Choose an image (.png)",
                BrowseFor::Texture => "‹ Choose a texture (.png)",
            };
            let mut rows = match self.browser.as_mut() {
                Some(b) => b.draw(title, w, h),
                None => Vec::new(),
            };
            if let (Some(m), true) = (&self.message, h >= 3) {
                let last = rows.len().saturating_sub(2);
                rows[last] = format!("\x1b[31m{}\x1b[0m", fit(&format!(" {m}"), w));
            }
            return rows;
        }

        let (title, subtitle) = match self.page {
            Page::Main => ("cubicle", "  ASCII 3D in your terminal"),
            _ => ("‹ Render settings", ""),
        };
        let n = self.item_count();
        let visible = h.saturating_sub(HEADER + FOOTER).max(1);
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + visible {
            self.top = self.sel + 1 - visible;
        }

        let head: String = format!(" {title}").chars().take(w).collect();
        let room = w.saturating_sub(head.chars().count());
        let sub: String = subtitle.chars().take(room).collect();
        let mut rows = vec![
            String::new(),
            format!("\x1b[1m{head}\x1b[0m\x1b[2m{sub}\x1b[0m"),
            format!("\x1b[2m{}\x1b[0m", "─".repeat(w)),
        ];
        for i in self.top..(self.top + visible).min(n) {
            let text = match (&self.page, MAIN_ITEMS.get(i)) {
                (Page::Main, Some(item)) => format!(" {}  {}", i + 1, item),
                _ => {
                    let (label, value) = self.setting_line(i);
                    if value.is_empty() {
                        format!("   {label}")
                    } else {
                        format!("   {}  {}", fit(label, 12), value)
                    }
                }
            };
            let line = fit(&text, w);
            rows.push(if i == self.sel {
                format!("\x1b[7m{line}\x1b[0m")
            } else {
                line
            });
        }
        while rows.len() < HEADER + visible {
            rows.push(String::new());
        }
        let (context, help) = match self.page {
            Page::Main => (
                self.summary(),
                if w >= 50 {
                    " ↑↓ move · Enter select · 1-5 jump · q quit"
                } else {
                    " ↑↓ · Enter · q"
                },
            ),
            _ => (
                self.setting_help(self.sel),
                if w >= 50 {
                    " ↑↓ move · ←→ change · Enter · Esc back"
                } else {
                    " ↑↓ ←→ · Esc"
                },
            ),
        };
        rows.push(match &self.message {
            Some(m) => format!("\x1b[31m{}\x1b[0m", fit(&format!(" {m}"), w)),
            None => format!("\x1b[2m{}\x1b[0m", fit(&format!(" {context}"), w)),
        });
        rows.push(format!("\x1b[2m{}\x1b[0m", fit(help, w)));
        rows.truncate(h.max(1));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::Mode;

    fn ok_probe() -> Result<GpuInfo, String> {
        Ok(GpuInfo {
            name: "Test GPU".into(),
            kind: "discrete GPU".into(),
            api: "Vulkan".into(),
        })
    }
    fn no_probe() -> Result<GpuInfo, String> {
        Err("no adapter".into())
    }

    fn menu() -> Menu {
        Menu::with_probe(Settings::default(), None, ok_probe)
    }

    fn plain(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c == '\x1b' {
                for d in it.by_ref() {
                    if d == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn press(m: &mut Menu, keys: &[KeyCode]) -> Action {
        let mut last = Action::None;
        for k in keys {
            last = m.on_key(*k);
        }
        last
    }

    #[test]
    fn main_menu_navigation_and_launch() {
        let mut m = menu();
        assert_eq!(press(&mut m, &[KeyCode::Enter]), Action::Launch(Launch::Cube));
        assert_eq!(press(&mut m, &[KeyCode::Char('5')]), Action::Quit);
        assert_eq!(menu().on_key(KeyCode::Esc), Action::Quit);
        // Down x4 then Enter = Quit; Up/Down clamp at the ends.
        let mut m = menu();
        assert_eq!(press(&mut m, &[KeyCode::Down; 9]), Action::None);
        assert_eq!(m.on_key(KeyCode::Enter), Action::Quit);
        assert_eq!(press(&mut m, &[KeyCode::Up; 9]), Action::None);
        assert_eq!(m.on_key(KeyCode::Enter), Action::Launch(Launch::Cube));
    }

    #[test]
    fn settings_cycle_and_persist() {
        let dir = std::env::temp_dir().join(format!("cubicle-menu-{}", std::process::id()));
        let path = dir.join("s.conf");
        let mut m = Menu::with_probe(Settings::default(), Some(path.clone()), ok_probe);
        press(&mut m, &[KeyCode::Char('4')]); // Render settings
        assert!(
            matches!(m.gpu, GpuStatus::Ready(_)),
            "opening settings probes the GPU"
        );
        // Row 0 = backend: Right cycles cpu -> gpu -> cpu+gpu -> cpu; Left goes back.
        m.on_key(KeyCode::Right);
        assert_eq!(m.settings.backend, Backend::Gpu);
        m.on_key(KeyCode::Right);
        assert_eq!(m.settings.backend, Backend::Both);
        m.on_key(KeyCode::Left);
        assert_eq!(m.settings.backend, Backend::Gpu);
        // Row 1 = mode, 2 = palette, 3 = colours, 4 = shading, 5 = faces, 6 = auto-spin.
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(m.settings.mode, Mode::AsciiColor);
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(m.settings.palette, Palette::Green);
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(m.settings.depth, Some(ColorDepth::TrueColor));
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert!(!m.settings.smooth);
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert!(m.settings.two_sided);
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert!(!m.settings.auto);
        // Every change was written to disk.
        let saved = Settings::load_from(&path);
        assert_eq!(saved, m.settings);
        assert_eq!(saved.backend, Backend::Gpu);
        // Down x2 -> Back; Enter returns to the main menu.
        press(&mut m, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(m.on_key(KeyCode::Char('1')), Action::Launch(Launch::Cube));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unavailable_gpu_is_reported_not_hidden() {
        let mut m = Menu::with_probe(Settings::default(), None, no_probe);
        press(&mut m, &[KeyCode::Char('4'), KeyCode::Right]); // backend = gpu
        let rows = m
            .draw(80, 24)
            .iter()
            .map(|r| plain(r))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("gpu (no gpu)"), "{rows}");
        assert!(rows.contains("GPU unavailable: no adapter"), "{rows}");
        let mut m = menu();
        press(&mut m, &[KeyCode::Char('4')]);
        let rows = m
            .draw(80, 24)
            .iter()
            .map(|r| plain(r))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("GPU: Test GPU (discrete GPU, Vulkan)"), "{rows}");
    }

    #[test]
    fn texture_row_opens_the_png_browser_and_picks() {
        let root = std::env::temp_dir().join(format!("cubicle-menu-tex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("t.png"), "x").unwrap();
        std::fs::write(root.join("m.obj"), "x").unwrap();
        let mut m = Menu::with_probe(
            Settings {
                browse_dir: Some(root.clone()),
                ..Settings::default()
            },
            None,
            ok_probe,
        );
        press(&mut m, &[KeyCode::Char('4')]);
        press(&mut m, &[KeyCode::Char('7')]); // not a hotkey in settings: ignored
        for _ in 0..7 {
            m.on_key(KeyCode::Down);
        }
        m.on_key(KeyCode::Enter); // Texture row: open the browser
        let rows = m
            .draw(60, 12)
            .iter()
            .map(|r| plain(r))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("Choose a texture"), "{rows}");
        assert!(
            rows.contains("t.png") && !rows.contains("m.obj"),
            "png filter: {rows}"
        );
        // ".." is first; Down to t.png and choose it.
        press(&mut m, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(
            m.settings.texture,
            Some(root.canonicalize().unwrap().join("t.png"))
        );
        // Back on the settings page, 'x' removes it.
        m.on_key(KeyCode::Char('x'));
        assert_eq!(m.settings.texture, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn model_browser_launches_and_remembers_the_folder() {
        let root = std::env::temp_dir().join(format!("cubicle-menu-obj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.obj"), "x").unwrap();
        let mut m = Menu::with_probe(
            Settings {
                browse_dir: Some(root.clone()),
                ..Settings::default()
            },
            None,
            ok_probe,
        );
        m.on_key(KeyCode::Char('2'));
        // entries: "..", "sub" -> open sub, then "..", "a.obj".
        press(&mut m, &[KeyCode::Down, KeyCode::Enter, KeyCode::Down]);
        let action = m.on_key(KeyCode::Enter);
        let canon = root.canonicalize().unwrap();
        assert_eq!(action, Action::Launch(Launch::Model(canon.join("sub/a.obj"))));
        assert_eq!(m.settings.browse_dir, Some(canon.join("sub")));
        // Esc in a browser returns to the main menu with the item selected.
        let mut m = menu();
        m.on_key(KeyCode::Char('2'));
        m.on_key(KeyCode::Esc);
        assert_eq!(
            m.on_key(KeyCode::Enter),
            Action::None,
            "now on item 2 (browse), which reopens the browser"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn touch_on_the_main_menu_and_settings() {
        let down = MouseEventKind::Down(MouseButton::Left);
        let mut m = menu();
        // Item rows start at row 3. Row 3 = cube.
        assert_eq!(m.on_mouse(down, 5, 3, 80, 24), Action::Launch(Launch::Cube));
        assert_eq!(m.on_mouse(down, 5, 7, 80, 24), Action::Quit);
        assert_eq!(
            m.on_mouse(down, 5, 1, 80, 24),
            Action::None,
            "title taps do nothing here"
        );
        // Settings: tap row 3 (Backend) on the right side steps forward, the left third steps back.
        m.on_mouse(down, 5, 6, 80, 24); // "Render settings"
        m.on_mouse(down, 70, 3, 80, 24);
        assert_eq!(m.settings.backend, Backend::Gpu);
        m.on_mouse(down, 2, 3, 80, 24);
        assert_eq!(m.settings.backend, Backend::Cpu);
        // The Back row and the title both return to the main menu.
        m.on_mouse(down, 10, 3 + ROW_BACK as u16, 80, 24);
        assert_eq!(m.on_mouse(down, 5, 3, 80, 24), Action::Launch(Launch::Cube));
        m.on_mouse(down, 5, 6, 80, 24);
        m.on_mouse(down, 5, 1, 80, 24);
        assert_eq!(m.on_mouse(down, 5, 3, 80, 24), Action::Launch(Launch::Cube));
    }

    #[test]
    fn drawing_always_fits_the_terminal() {
        for page_keys in [vec![], vec![KeyCode::Char('4')], vec![KeyCode::Char('2')]] {
            for (w, h) in [(80, 24), (45, 14), (30, 8), (20, 5), (10, 3), (1, 1)] {
                let mut m = menu();
                press(&mut m, &page_keys);
                m.show_message("a fairly long error message that must be clipped to the width");
                let rows = m.draw(w, h);
                assert!(rows.len() <= h.max(1), "{w}x{h}: {} rows", rows.len());
                for r in &rows {
                    assert!(plain(r).chars().count() <= w, "{w}x{h}: {:?}", plain(r));
                }
            }
        }
    }

    #[test]
    fn small_screens_scroll_to_keep_the_selection_visible() {
        let mut m = menu();
        press(&mut m, &[KeyCode::Char('4')]);
        press(&mut m, &[KeyCode::End]);
        let rows = m.draw(60, 7); // only 2 list rows fit
        assert!(
            rows.iter().any(|r| plain(r).contains("Back")),
            "selected last row must be shown: {rows:?}"
        );
    }

    #[test]
    fn messages_clear_on_the_next_key() {
        let mut m = menu();
        m.show_message("boom");
        assert!(m.draw(60, 12).iter().any(|r| r.contains("boom")));
        m.on_key(KeyCode::Down);
        assert!(!m.draw(60, 12).iter().any(|r| r.contains("boom")));
    }
}

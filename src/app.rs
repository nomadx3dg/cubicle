//! Interactive loops (3D model viewer, image viewer) and the one-shot renderer.

use crate::cli::{Config, Input};
use crate::display::{hud, hud_hit, ColorDepth, Mode, Palette};
use crate::image::Image;
use crate::mesh::Mesh;
use crate::obj;
use crate::raster::{render_textured, Frame, RenderOpts};
use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute, terminal,
};
use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::{Duration, Instant};

pub const STEP: f32 = 0.6; // radians per tap
pub const CAM_MIN: f32 = 2.6;
pub const CAM_MAX: f32 = 20.0;
pub const CAM_START: f32 = 5.0;
const START_ANGLES: (f32, f32) = (0.4, 0.6);

/// Puts the terminal into raw/alternate-screen mode and always restores it.
struct TermGuard;

impl TermGuard {
    fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        )?;
        // Disable line wrap so drawing the last cell never scrolls the screen.
        print!("\x1b[?7l");
        io::stdout().flush()?;
        Ok(TermGuard)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        print!("\x1b[?7h");
        let _ = execute!(
            io::stdout(),
            cursor::Show,
            DisableMouseCapture,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// Redraws only the rows that changed, inside a synchronized update.
struct Screen {
    prev: Vec<String>,
    last_size: (u16, u16),
    out: String,
}

impl Screen {
    fn new() -> Self {
        Screen {
            prev: Vec::new(),
            last_size: (0, 0),
            out: String::new(),
        }
    }

    fn draw(&mut self, size: (u16, u16), rows: Vec<String>) -> io::Result<()> {
        self.out.clear();
        self.out.push_str("\x1b[?2026h");
        if size != self.last_size {
            self.out.push_str("\x1b[2J");
            self.prev.clear();
            self.last_size = size;
        }
        for (i, row) in rows.iter().enumerate() {
            if self.prev.get(i) != Some(row) {
                let _ = write!(self.out, "\x1b[{};1H{}", i + 1, row);
            }
        }
        self.out.push_str("\x1b[?2026l");
        self.prev = rows;
        let mut stdout = io::stdout().lock();
        stdout.write_all(self.out.as_bytes())?;
        stdout.flush()
    }
}

// ---------------------------------------------------------------- model view

/// What the user is looking at and how. Input handlers only mutate this, which
/// keeps them unit-testable without a terminal.
pub struct ModelView {
    pub tax: f32,
    pub tay: f32,
    pub ax: f32,
    pub ay: f32,
    pub cam: f32,
    pub auto: bool,
    pub mode: Mode,
    pub palette: Palette,
    pub depth: ColorDepth,
    pub smooth: bool,
    pub two_sided: bool,
    /// Texture for models with `vt` coordinates, and whether it is shown.
    pub texture: Option<Image>,
    pub tex_on: bool,
    drag_from: Option<(u16, u16)>,
    start: (f32, f32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

impl ModelView {
    pub fn new(cfg: &Config) -> Self {
        let (ax, ay) = cfg.rot.unwrap_or(START_ANGLES);
        ModelView {
            tax: ax,
            tay: ay,
            ax,
            ay,
            cam: CAM_START,
            auto: cfg.auto,
            mode: cfg.mode,
            palette: cfg.palette,
            depth: cfg.depth,
            smooth: cfg.smooth,
            two_sided: cfg.two_sided,
            texture: None,
            tex_on: true,
            drag_from: None,
            start: (ax, ay),
        }
    }

    pub fn zoom_in(&mut self, d: f32) {
        self.cam = (self.cam - d).max(CAM_MIN);
    }
    pub fn zoom_out(&mut self, d: f32) {
        self.cam = (self.cam + d).min(CAM_MAX);
    }

    pub fn on_key(&mut self, code: KeyCode) -> Flow {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Flow::Quit,
            KeyCode::Left => self.tay += STEP / 2.0,
            KeyCode::Right => self.tay -= STEP / 2.0,
            KeyCode::Up => self.tax += STEP / 2.0,
            KeyCode::Down => self.tax -= STEP / 2.0,
            KeyCode::Char('+') | KeyCode::Char('=') => self.zoom_in(0.3),
            KeyCode::Char('-') => self.zoom_out(0.3),
            KeyCode::Char(' ') => self.auto = !self.auto,
            KeyCode::Char('m') => self.mode = self.mode.next(),
            KeyCode::Char('c') => self.palette = self.palette.next(),
            KeyCode::Char('s') => self.smooth = !self.smooth,
            KeyCode::Char('b') => self.two_sided = !self.two_sided,
            KeyCode::Char('t') if self.texture.is_some() => self.tex_on = !self.tex_on,
            KeyCode::Char('r') => {
                self.tax = self.start.0;
                self.tay = self.start.1;
                self.cam = CAM_START;
            }
            _ => {}
        }
        Flow::Continue
    }

    pub fn hud_labels(&self) -> [&'static str; 4] {
        [
            "[ - ]",
            if self.auto { "auto: on" } else { "auto: off" },
            match self.mode {
                Mode::Ascii => "mode: ascii",
                Mode::AsciiColor => "mode: color",
                Mode::Blocks => "mode: blocks",
            },
            "[ + ]",
        ]
    }

    /// Touch (Termux) and mouse. A tap arrives as a left click and an up/down
    /// swipe arrives as the scroll wheel. Termux does not report sideways finger
    /// movement, so sideways turning is done with taps.
    pub fn on_mouse(&mut self, kind: MouseEventKind, col: u16, row: u16, tw: u16, th: u16) {
        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if row + 1 >= th {
                    // Bottom bar: [ - ] [ auto ] [ mode ] [ + ]
                    match hud_hit(col as usize, tw as usize, 4) {
                        0 => self.zoom_out(0.6),
                        1 => self.auto = !self.auto,
                        2 => self.mode = self.mode.next(),
                        _ => self.zoom_in(0.6),
                    }
                    self.drag_from = None;
                } else {
                    // Tap the side you want the cube to turn toward.
                    let nx = (col as f32 + 0.5 - tw as f32 / 2.0) / (tw as f32 / 2.0);
                    let ny = (row as f32 + 0.5 - th as f32 / 2.0) / (th as f32 / 2.0);
                    if nx.abs() < 0.2 && ny.abs() < 0.2 {
                        self.auto = !self.auto;
                    } else {
                        self.auto = false;
                        if nx.abs() >= ny.abs() {
                            self.tay -= nx.signum() * STEP;
                        } else {
                            self.tax -= ny.signum() * STEP;
                        }
                    }
                    self.drag_from = Some((col, row));
                }
            }
            // Real mouse drag (not sent by Termux touch).
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((c, r)) = self.drag_from {
                    self.auto = false;
                    self.tay -= (col as f32 - c as f32) * 0.08;
                    self.tax -= (row as f32 - r as f32) * 0.12;
                    self.drag_from = Some((col, row));
                }
            }
            MouseEventKind::Up(_) => self.drag_from = None,
            MouseEventKind::ScrollDown => {
                self.auto = false;
                self.tax += STEP / 3.0;
            }
            MouseEventKind::ScrollUp => {
                self.auto = false;
                self.tax -= STEP / 3.0;
            }
            _ => {}
        }
    }

    /// Advances auto-spin and eases the shown angles toward the targets.
    pub fn step(&mut self, dt: f32) {
        if self.auto {
            self.tay += dt * 0.8;
            self.tax += dt * 0.35;
        }
        let ease = 1.0 - (-14.0 * dt).exp();
        self.ax += (self.tax - self.ax) * ease;
        self.ay += (self.tay - self.ay) * ease;
        for (t, a) in [(&mut self.tax, &mut self.ax), (&mut self.tay, &mut self.ay)] {
            if *t > TAU {
                *t -= TAU;
                *a -= TAU;
            } else if *t < -TAU {
                *t += TAU;
                *a += TAU;
            }
        }
    }

    fn opts(&self) -> RenderOpts {
        RenderOpts {
            smooth: self.smooth,
            two_sided: self.two_sided,
            pixels_per_row: self.mode.pixels_per_row(),
        }
    }

    /// Renders the 3D area (`rows` terminal rows tall) to encoded text rows.
    pub fn frame_rows(&self, mesh: &Mesh, cols: usize, rows: usize) -> Vec<String> {
        let ppr = self.mode.pixels_per_row();
        let mut frame = Frame::new(cols, rows * ppr);
        let tex = if self.tex_on { self.texture.as_ref() } else { None };
        render_textured(mesh, self.ax, self.ay, self.cam, self.opts(), tex, &mut frame);
        frame.canvas.encode_with(self.mode, self.palette, self.depth)
    }
}

fn load_mesh(cfg: &Config) -> Result<Mesh, String> {
    match &cfg.input {
        Input::Obj(p) => obj::load(p, cfg.crease),
        _ => Ok(Mesh::cube()),
    }
}

/// Builds the model view, loading `--texture` if given.
fn build_view(cfg: &Config) -> Result<ModelView, String> {
    let mut view = ModelView::new(cfg);
    if let Some(p) = &cfg.texture {
        view.texture = Some(Image::load(p)?);
    }
    Ok(view)
}

fn run_model(cfg: &Config) -> Result<(), String> {
    let mesh = load_mesh(cfg)?;
    let mut view = build_view(cfg)?;
    let _guard = TermGuard::new().map_err(|e| format!("cannot start terminal UI: {e}"))?;
    run_model_loop(&mesh, &mut view).map_err(|e| e.to_string())
}

fn run_model_loop(mesh: &Mesh, view: &mut ModelView) -> io::Result<()> {
    let mut screen = Screen::new();
    let mut last = Instant::now();
    let frame_time = Duration::from_millis(33);
    loop {
        let start = Instant::now();
        let dt = start.duration_since(last).as_secs_f32().clamp(0.0001, 0.1);
        last = start;
        let (tw, th) = terminal::size()?;

        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if view.on_key(k.code) == Flow::Quit {
                        return Ok(());
                    }
                }
                Event::Mouse(m) => view.on_mouse(m.kind, m.column, m.row, tw, th),
                _ => {}
            }
        }
        view.step(dt);

        let (w, h) = (tw as usize, (th as usize).saturating_sub(1).max(1));
        let mut rows = view.frame_rows(mesh, w, h);
        rows.push(hud(w, &view.hud_labels()));
        screen.draw((tw, th), rows)?;

        if let Some(rest) = frame_time.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

// ---------------------------------------------------------------- image view

pub struct ImageView {
    pub mode: Mode,
    pub palette: Palette,
    pub depth: ColorDepth,
    pub invert: bool,
}

impl ImageView {
    pub fn on_key(&mut self, code: KeyCode) -> Flow {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Flow::Quit,
            KeyCode::Char('m') => self.mode = self.mode.next(),
            KeyCode::Char('c') => self.palette = self.palette.next(),
            KeyCode::Char('i') => self.invert = !self.invert,
            _ => {}
        }
        Flow::Continue
    }

    pub fn hud_labels(&self) -> [String; 3] {
        [
            format!("mode: {}", self.mode.label()),
            format!("color: {}", self.palette.label()),
            format!("invert: {}", if self.invert { "on" } else { "off" }),
        ]
    }

    pub fn on_tap(&mut self, col: u16, row: u16, tw: u16, th: u16) {
        if row + 1 >= th {
            match hud_hit(col as usize, tw as usize, 3) {
                0 => self.mode = self.mode.next(),
                1 => self.palette = self.palette.next(),
                _ => self.invert = !self.invert,
            }
        }
    }

    pub fn frame_rows(&self, img: &Image, cols: usize, rows: usize) -> Vec<String> {
        img.to_canvas(cols, rows, self.mode.pixels_per_row(), self.invert)
            .encode_with(self.mode, self.palette, self.depth)
    }
}

fn run_image(cfg: &Config, img: &Image) -> Result<(), String> {
    let mut view = ImageView {
        mode: cfg.mode,
        palette: cfg.palette,
        depth: cfg.depth,
        invert: cfg.invert,
    };
    let _guard = TermGuard::new().map_err(|e| format!("cannot start terminal UI: {e}"))?;
    (|| -> io::Result<()> {
        let mut screen = Screen::new();
        let mut dirty = true;
        loop {
            // Static image: only redraw after input or a resize.
            if dirty {
                let (tw, th) = terminal::size()?;
                let (w, h) = (tw as usize, (th as usize).saturating_sub(1).max(1));
                let mut rows = view.frame_rows(img, w, h);
                let labels = view.hud_labels();
                let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                rows.push(hud(w, &refs));
                screen.draw((tw, th), rows)?;
                dirty = false;
            }
            if event::poll(Duration::from_millis(250))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        if view.on_key(k.code) == Flow::Quit {
                            return Ok(());
                        }
                        dirty = true;
                    }
                    Event::Mouse(m) => {
                        let (tw, th) = terminal::size()?;
                        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                            view.on_tap(m.column, m.row, tw, th);
                            dirty = true;
                        }
                    }
                    Event::Resize(..) => dirty = true,
                    _ => {}
                }
            }
        }
    })()
    .map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ one-shot

/// Terminal size, falling back to 80x24 when stdout is not a terminal.
fn default_size() -> (usize, usize) {
    terminal::size()
        .map(|(w, h)| (w as usize, h as usize))
        .unwrap_or((80, 24))
}

/// Renders a single frame (no HUD) as text. Used by `--once` and by tests.
pub fn render_once(cfg: &Config) -> Result<String, String> {
    let (w, h) = cfg.size.unwrap_or_else(default_size);
    let rows = match &cfg.input {
        Input::Png(p) => {
            let img = Image::load(p)?;
            img.to_canvas(w, h, cfg.mode.pixels_per_row(), cfg.invert)
                .encode_with(cfg.mode, cfg.palette, cfg.depth)
        }
        _ => {
            let mesh = load_mesh(cfg)?;
            build_view(cfg)?.frame_rows(&mesh, w, h)
        }
    };
    let mut s = rows.join("\n");
    s.push('\n');
    Ok(s)
}

/// Entry point used by `main`.
pub fn run(cfg: &Config) -> Result<(), String> {
    if cfg.once {
        let text = render_once(cfg)?;
        return io::stdout().write_all(text.as_bytes()).map_err(|e| e.to_string());
    }
    match &cfg.input {
        Input::Png(p) => {
            let img = Image::load(p)?;
            run_image(cfg, &img)
        }
        _ => run_model(cfg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ModelView {
        ModelView::new(&Config::default())
    }

    #[test]
    fn keys_adjust_state() {
        let mut v = view();
        let (t0, c0) = (v.tay, v.cam);
        v.on_key(KeyCode::Left);
        assert!(v.tay > t0);
        v.on_key(KeyCode::Char('+'));
        assert!(v.cam < c0);
        v.on_key(KeyCode::Char('m'));
        assert_eq!(v.mode, Mode::AsciiColor);
        v.on_key(KeyCode::Char('s'));
        assert!(!v.smooth);
        v.on_key(KeyCode::Char('b'));
        assert!(v.two_sided);
        assert_eq!(v.on_key(KeyCode::Char('q')), Flow::Quit);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = view();
        for _ in 0..200 {
            v.zoom_in(1.0);
        }
        assert_eq!(v.cam, CAM_MIN);
        for _ in 0..200 {
            v.zoom_out(1.0);
        }
        assert_eq!(v.cam, CAM_MAX);
    }

    #[test]
    fn reset_restores_view() {
        let mut v = view();
        v.on_key(KeyCode::Left);
        v.on_key(KeyCode::Char('-'));
        v.on_key(KeyCode::Char('r'));
        assert_eq!((v.tax, v.tay, v.cam), (START_ANGLES.0, START_ANGLES.1, CAM_START));
    }

    #[test]
    fn touch_zones_on_an_80x24_terminal() {
        let down = MouseEventKind::Down(MouseButton::Left);
        // Bottom bar, quarters: zoom out, auto, mode, zoom in.
        let mut v = view();
        v.on_mouse(down, 5, 23, 80, 24);
        assert!(v.cam > CAM_START);
        v.on_mouse(down, 30, 23, 80, 24);
        assert!(!v.auto);
        v.on_mouse(down, 50, 23, 80, 24);
        assert_eq!(v.mode, Mode::AsciiColor);
        v.on_mouse(down, 75, 23, 80, 24);
        assert_eq!(v.cam, CAM_START);
        // Centre toggles auto; right edge turns the cube.
        let mut v = view();
        v.on_mouse(down, 40, 11, 80, 24);
        assert!(!v.auto);
        let t = v.tay;
        v.on_mouse(down, 78, 11, 80, 24);
        assert!(v.tay < t);
    }

    #[test]
    fn step_wraps_angles_without_a_visible_jump() {
        let mut v = view();
        v.tay = TAU + 0.1;
        v.ay = TAU + 0.05;
        v.step(0.0001);
        assert!(v.tay < TAU && (v.tay - v.ay).abs() < 0.2);
    }

    #[test]
    fn once_renders_every_mode() {
        for mode in [Mode::Ascii, Mode::AsciiColor, Mode::Blocks] {
            let cfg = Config {
                once: true,
                size: Some((40, 12)),
                mode,
                ..Config::default()
            };
            let text = render_once(&cfg).unwrap();
            assert_eq!(text.lines().count(), 12);
            assert!(text.trim().len() > 20);
        }
    }

    #[test]
    fn texture_toggle_only_works_with_a_texture() {
        let mut v = view();
        v.on_key(KeyCode::Char('t'));
        assert!(v.tex_on, "no texture loaded: 't' does nothing");
        v.texture = Some(Image {
            w: 1,
            h: 1,
            rgb: vec![[0, 0, 0]],
        });
        v.on_key(KeyCode::Char('t'));
        assert!(!v.tex_on);
    }

    #[test]
    fn once_reports_missing_files() {
        let cfg = Config {
            input: Input::Obj("/nonexistent/x.obj".into()),
            once: true,
            ..Config::default()
        };
        assert!(render_once(&cfg).unwrap_err().contains("cannot read"));
    }
}

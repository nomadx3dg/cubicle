//! cubicle: a tiny CPU 3D rasteriser that draws to the terminal as ASCII.
//! Keyboard, mouse and touch (Termux) controls.

use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute, terminal,
};
use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::{Duration, Instant};

// Dark -> bright. Add or swap characters to change the look.
const RAMP: &[u8] = b" .,:;-=+*#%@";

const STEP: f32 = 0.6; // radians per tap
const CAM_MIN: f32 = 2.6;
const CAM_MAX: f32 = 20.0;

#[derive(Clone, Copy)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

impl V3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        V3 { x, y, z }
    }
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn scale(self, k: f32) -> V3 {
        V3::new(self.x * k, self.y * k, self.z * k)
    }
    fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: V3) -> V3 {
        V3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn normalize(self) -> V3 {
        let l = self.dot(self).sqrt();
        if l == 0.0 {
            self
        } else {
            self.scale(1.0 / l)
        }
    }
    fn rot_x(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(self.x, self.y * c - self.z * s, self.y * s + self.z * c)
    }
    fn rot_y(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(self.x * c + self.z * s, self.y, -self.x * s + self.z * c)
    }
}

type Tri = [V3; 3];

/// Unit cube as 12 triangles. Winding is fixed up so normals point outward.
fn cube() -> Vec<Tri> {
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
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 3, 7, 4],
        [1, 5, 6, 2],
        [0, 4, 5, 1],
        [3, 2, 6, 7],
    ];
    let mut tris = Vec::new();
    for q in quads {
        for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
            tris.push(orient_outward([v[t[0]], v[t[1]], v[t[2]]]));
        }
    }
    tris
}

/// For a mesh centred on the origin, flip any triangle whose normal faces inward.
/// (Fine for the cube. For .obj models you'll want proper winding instead.)
fn orient_outward(t: Tri) -> Tri {
    let n = t[1].sub(t[0]).cross(t[2].sub(t[0]));
    let centroid = t[0].add(t[1]).add(t[2]).scale(1.0 / 3.0);
    if n.dot(centroid) < 0.0 {
        [t[0], t[2], t[1]]
    } else {
        t
    }
}

struct Frame {
    w: usize,
    h: usize,
    shade: Vec<f32>, // 0..1 brightness, negative = empty
    depth: Vec<f32>, // stores 1/z, larger = closer
}

impl Frame {
    fn new(w: usize, h: usize) -> Self {
        Frame {
            w,
            h,
            shade: vec![-1.0; w * h],
            depth: vec![0.0; w * h],
        }
    }

    /// Rasterise one triangle. `view` are the vertices in camera space, `n` the face
    /// normal. Lighting is a point light, evaluated per pixel for smooth gradients.
    fn triangle(&mut self, p: [(f32, f32); 3], inv_z: [f32; 3], view: [V3; 3], n: V3, light: V3) {
        let edge = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
            (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
        };
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-6 {
            return;
        }
        let min_x = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0);
        let max_x = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().min(self.w as f32 - 1.0);
        let min_y = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0);
        let max_y = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().min(self.h as f32 - 1.0);
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
                let z = w0 * inv_z[0] + w1 * inv_z[1] + w2 * inv_z[2];
                let i = y * self.w + x;
                if z <= self.depth[i] {
                    continue;
                }
                self.depth[i] = z;
                // Perspective-correct position of this pixel in camera space.
                let (a0, a1, a2) = (w0 * inv_z[0], w1 * inv_z[1], w2 * inv_z[2]);
                let pos = view[0]
                    .scale(a0)
                    .add(view[1].scale(a1))
                    .add(view[2].scale(a2))
                    .scale(1.0 / (a0 + a1 + a2));
                let to_light = light.sub(pos);
                let d2 = to_light.dot(to_light);
                let diff = n.dot(to_light.normalize()).max(0.0);
                let atten = 1.0 / (1.0 + 0.06 * d2);
                self.shade[i] = (0.08 + 1.9 * diff * atten).min(1.0);
            }
        }
    }

    fn rows(&self) -> Vec<String> {
        self.shade
            .chunks(self.w)
            .map(|row| {
                row.iter()
                    .map(|&b| {
                        if b < 0.0 {
                            ' '
                        } else {
                            RAMP[(b.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32 + 0.5) as usize]
                                as char
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

/// Draws the mesh into the frame. `cam` is the camera distance on the -z axis.
fn render(mesh: &[Tri], ax: f32, ay: f32, cam: f32, frame: &mut Frame) {
    let eye = V3::new(0.0, 0.0, -cam);
    let light = V3::new(2.5, 3.0, -cam + 0.5);
    // Size the cube so it fits both tall and wide screens. Cells are ~2x taller than wide.
    let focal = (frame.h as f32 * 1.1).min(frame.w as f32 * 0.65) * cam / 5.0;
    let (cx, cy) = (frame.w as f32 / 2.0, frame.h as f32 / 2.0);

    for tri in mesh {
        let t = [
            tri[0].rot_y(ay).rot_x(ax),
            tri[1].rot_y(ay).rot_x(ax),
            tri[2].rot_y(ay).rot_x(ax),
        ];
        let n = t[1].sub(t[0]).cross(t[2].sub(t[0])).normalize();
        // Backface culling: skip faces pointing away from the camera.
        if n.dot(eye.sub(t[0])) <= 0.0 {
            continue;
        }
        let mut pts = [(0.0, 0.0); 3];
        let mut inv_z = [0.0; 3];
        let mut view = [V3::new(0.0, 0.0, 0.0); 3];
        let mut behind = false;
        for (i, v) in t.iter().enumerate() {
            let z = v.z + cam;
            if z <= 0.1 {
                behind = true;
                break;
            }
            pts[i] = (cx + v.x * focal / z * 2.0, cy - v.y * focal / z);
            inv_z[i] = 1.0 / z;
            view[i] = *v;
        }
        if behind {
            continue;
        }
        frame.triangle(pts, inv_z, view, n, light);
    }
}

/// Bottom bar with three touch buttons: zoom out, auto-spin, zoom in.
fn hud(w: usize, auto: bool) -> String {
    let third = w / 3;
    let mid = if auto { "auto: on" } else { "auto: off" };
    let cell = |s: &str, width: usize| {
        let s: String = s.chars().take(width).collect();
        let pad = width - s.chars().count();
        let l = pad / 2;
        format!("{}{}{}", " ".repeat(l), s, " ".repeat(pad - l))
    };
    format!(
        "{}{}{}",
        cell("[ - ]", third),
        cell(mid, w - 2 * third),
        cell("[ + ]", third)
    )
}

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

fn main() -> io::Result<()> {
    let _guard = TermGuard::new()?;
    let mesh = cube();
    let (mut tax, mut tay) = (0.4_f32, 0.6_f32); // target angles (input changes these)
    let (mut ax, mut ay) = (tax, tay); // shown angles (ease toward targets)
    let mut cam = 5.0_f32;
    let mut auto = true;
    let mut drag_from: Option<(u16, u16)> = None;
    let mut prev: Vec<String> = Vec::new();
    let mut last_size = (0u16, 0u16);
    let mut out = String::new();
    let mut last = Instant::now();
    let frame_time = Duration::from_millis(33);

    loop {
        let start = Instant::now();
        let dt = start.duration_since(last).as_secs_f32().clamp(0.0001, 0.1);
        last = start;
        let (tw, th) = terminal::size()?;

        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Left => tay += STEP / 2.0,
                    KeyCode::Right => tay -= STEP / 2.0,
                    KeyCode::Up => tax += STEP / 2.0,
                    KeyCode::Down => tax -= STEP / 2.0,
                    KeyCode::Char('+') | KeyCode::Char('=') => cam = (cam - 0.3).max(CAM_MIN),
                    KeyCode::Char('-') => cam = (cam + 0.3).min(CAM_MAX),
                    KeyCode::Char(' ') => auto = !auto,
                    _ => {}
                },
                // Touch (Termux) and mouse. A tap arrives as a left click and an
                // up/down swipe arrives as the scroll wheel. Termux does not report
                // sideways finger movement, so sideways turning is done with taps.
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if m.row + 1 >= th {
                            // Bottom bar: [ - ] [ auto ] [ + ]
                            let third = tw / 3;
                            if m.column < third {
                                cam = (cam + 0.6).min(CAM_MAX);
                            } else if m.column >= tw - third {
                                cam = (cam - 0.6).max(CAM_MIN);
                            } else {
                                auto = !auto;
                            }
                            drag_from = None;
                        } else {
                            // Tap the side you want the cube to turn toward.
                            let nx = (m.column as f32 + 0.5 - tw as f32 / 2.0) / (tw as f32 / 2.0);
                            let ny = (m.row as f32 + 0.5 - th as f32 / 2.0) / (th as f32 / 2.0);
                            if nx.abs() < 0.2 && ny.abs() < 0.2 {
                                auto = !auto;
                            } else {
                                auto = false;
                                if nx.abs() >= ny.abs() {
                                    tay -= nx.signum() * STEP;
                                } else {
                                    tax -= ny.signum() * STEP;
                                }
                            }
                            drag_from = Some((m.column, m.row));
                        }
                    }
                    // Real mouse drag (not sent by Termux touch).
                    MouseEventKind::Drag(MouseButton::Left) => {
                        if let Some((c, r)) = drag_from {
                            auto = false;
                            tay -= (m.column as f32 - c as f32) * 0.08;
                            tax -= (m.row as f32 - r as f32) * 0.12;
                            drag_from = Some((m.column, m.row));
                        }
                    }
                    MouseEventKind::Up(_) => drag_from = None,
                    // Swipe the cube up or down.
                    MouseEventKind::ScrollDown => {
                        auto = false;
                        tax += STEP / 3.0;
                    }
                    MouseEventKind::ScrollUp => {
                        auto = false;
                        tax -= STEP / 3.0;
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if auto {
            tay += dt * 0.8;
            tax += dt * 0.35;
        }
        // Ease the shown angles toward the targets so taps glide instead of jumping.
        let ease = 1.0 - (-14.0 * dt).exp();
        ax += (tax - ax) * ease;
        ay += (tay - ay) * ease;
        for (t, a) in [(&mut tax, &mut ax), (&mut tay, &mut ay)] {
            if *t > std::f32::consts::TAU {
                *t -= std::f32::consts::TAU;
                *a -= std::f32::consts::TAU;
            } else if *t < -std::f32::consts::TAU {
                *t += std::f32::consts::TAU;
                *a += std::f32::consts::TAU;
            }
        }

        let (w, h) = (tw as usize, (th as usize).saturating_sub(1).max(1));
        let mut frame = Frame::new(w, h);
        render(&mesh, ax, ay, cam, &mut frame);
        let mut rows = frame.rows();
        rows.push(hud(w, auto));

        // Only redraw rows that changed, inside a synchronized update (no flicker/tearing).
        out.clear();
        out.push_str("\x1b[?2026h");
        if (tw, th) != last_size {
            out.push_str("\x1b[2J");
            prev.clear();
            last_size = (tw, th);
        }
        for (i, row) in rows.iter().enumerate() {
            if prev.get(i) != Some(row) {
                let _ = write!(out, "\x1b[{};1H{}", i + 1, row);
            }
        }
        out.push_str("\x1b[?2026l");
        prev = rows;

        let mut stdout = io::stdout().lock();
        stdout.write_all(out.as_bytes())?;
        stdout.flush()?;
        drop(stdout);

        if let Some(rest) = frame_time.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

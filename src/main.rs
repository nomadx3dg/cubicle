//! cubicle: a tiny CPU 3D rasteriser that draws to the terminal as ASCII.
//! Keyboard, mouse and touch (Termux) controls. Milestone 1: a spinning, lit cube.

use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute, terminal,
};
use std::io::{self, Write};
use std::time::{Duration, Instant};

// Dark -> bright. Add or swap characters to change the look.
const RAMP: &[u8] = b" .:-=+*#%@";

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
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
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
            V3::new(self.x / l, self.y / l, self.z / l)
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
    let centroid = V3::new(
        (t[0].x + t[1].x + t[2].x) / 3.0,
        (t[0].y + t[1].y + t[2].y) / 3.0,
        (t[0].z + t[1].z + t[2].z) / 3.0,
    );
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

    fn triangle(&mut self, p: [(f32, f32); 3], inv_z: [f32; 3], brightness: f32) {
        let edge = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
            (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
        };
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-6 {
            return;
        }
        let min_x = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let max_x = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().min(self.w as f32 - 1.0);
        let min_y = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let max_y = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().min(self.h as f32 - 1.0);
        if max_x < 0.0 || max_y < 0.0 {
            return;
        }
        for y in min_y..=(max_y as usize) {
            for x in min_x..=(max_x as usize) {
                let pt = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(p[1], p[2], pt) / area;
                let w1 = edge(p[2], p[0], pt) / area;
                let w2 = edge(p[0], p[1], pt) / area;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * inv_z[0] + w1 * inv_z[1] + w2 * inv_z[2];
                let i = y * self.w + x;
                if z > self.depth[i] {
                    self.depth[i] = z;
                    self.shade[i] = brightness;
                }
            }
        }
    }

    fn to_ascii(&self, out: &mut String) {
        for row in self.shade.chunks(self.w) {
            for &b in row {
                let c = if b < 0.0 {
                    ' '
                } else {
                    RAMP[(b.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32) as usize] as char
                };
                out.push(c);
            }
            out.push_str("\r\n");
        }
    }
}

/// Draws the mesh into the frame. `cam` is the camera distance on the -z axis.
fn render(mesh: &[Tri], ax: f32, ay: f32, cam: f32, frame: &mut Frame) {
    let light = V3::new(0.4, 0.7, -0.6).normalize();
    let eye = V3::new(0.0, 0.0, -cam);
    let focal = frame.h as f32 * 1.1;
    let (cx, cy) = (frame.w as f32 / 2.0, frame.h as f32 / 2.0);

    for tri in mesh {
        let t: Vec<V3> = tri.iter().map(|v| v.rot_y(ay).rot_x(ax)).collect();
        let n = t[1].sub(t[0]).cross(t[2].sub(t[0])).normalize();
        // Backface culling: skip faces pointing away from the camera.
        if n.dot(eye.sub(t[0])) <= 0.0 {
            continue;
        }
        let mut pts = [(0.0, 0.0); 3];
        let mut inv_z = [0.0; 3];
        let mut behind = false;
        for (i, v) in t.iter().enumerate() {
            let z = v.z + cam;
            if z <= 0.1 {
                behind = true;
                break;
            }
            // Terminal cells are about twice as tall as wide, so stretch x by 2.
            pts[i] = (cx + v.x * focal / z * 2.0, cy - v.y * focal / z);
            inv_z[i] = 1.0 / z;
        }
        if behind {
            continue;
        }
        let brightness = 0.1 + 0.9 * n.dot(light).max(0.0);
        frame.triangle(pts, inv_z, brightness);
    }
}

const TAP_KICK: f32 = 2.0; // rad/s given by a tap, decays quickly
const CAM_MIN: f32 = 2.6;
const CAM_MAX: f32 = 20.0;

/// Bottom bar with three touch buttons: zoom out, pause/play, zoom in.
fn hud(w: usize, paused: bool, fps: f32) -> String {
    let third = w / 3;
    let mid = if paused {
        "play".to_string()
    } else {
        format!("pause {:.0}fps", fps)
    };
    let cell = |s: &str, width: usize| {
        let s: String = s.chars().take(width).collect();
        let pad = width - s.chars().count();
        let l = pad / 2;
        format!("{}{}{}", " ".repeat(l), s, " ".repeat(pad - l))
    };
    format!(
        "{}{}{}",
        cell("[ - ]", third),
        cell(&mid, w - 2 * third),
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
        Ok(TermGuard)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
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
    let (mut ax, mut ay, mut cam) = (0.4_f32, 0.0_f32, 5.0_f32);
    let (mut vx, mut vy) = (0.0_f32, 0.0_f32); // extra spin from taps
    let mut drag_from: Option<(u16, u16)> = None;
    let mut paused = false;
    let mut out = String::new();
    let mut last = Instant::now();
    let mut fps = 30.0_f32;
    let frame_time = Duration::from_millis(33);

    loop {
        let start = Instant::now();
        let dt = start.duration_since(last).as_secs_f32().max(0.0001);
        last = start;
        if dt > 0.005 {
            fps = fps * 0.9 + (1.0 / dt) * 0.1;
        }
        let (tw, th) = terminal::size()?;

        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Left => ay -= 0.1,
                    KeyCode::Right => ay += 0.1,
                    KeyCode::Up => ax -= 0.1,
                    KeyCode::Down => ax += 0.1,
                    KeyCode::Char('+') | KeyCode::Char('=') => cam = (cam - 0.3).max(CAM_MIN),
                    KeyCode::Char('-') => cam = (cam + 0.3).min(CAM_MAX),
                    KeyCode::Char(' ') => paused = !paused,
                    _ => {}
                },
                // Touch (Termux) and mouse. A tap arrives as a left click and a
                // vertical swipe arrives as the scroll wheel.
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if m.row + 1 >= th {
                            // Bottom bar: [ - ] [ pause ] [ + ]
                            let third = tw / 3;
                            if m.column < third {
                                cam = (cam + 0.6).min(CAM_MAX);
                            } else if m.column >= tw - third {
                                cam = (cam - 0.6).max(CAM_MIN);
                            } else {
                                paused = !paused;
                            }
                            drag_from = None;
                        } else {
                            // Tap the screen: center = pause, edges = spin that way.
                            let nx = (m.column as f32 - tw as f32 / 2.0) / (tw as f32 / 2.0);
                            let ny = (m.row as f32 - th as f32 / 2.0) / (th as f32 / 2.0);
                            if nx.abs() < 0.25 && ny.abs() < 0.25 {
                                paused = !paused;
                            } else if nx.abs() > ny.abs() {
                                vy += nx.signum() * TAP_KICK;
                            } else {
                                vx += ny.signum() * TAP_KICK;
                            }
                            drag_from = Some((m.column, m.row));
                        }
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        if let Some((c, r)) = drag_from {
                            ay += (m.column as f32 - c as f32) * 0.06;
                            ax += (m.row as f32 - r as f32) * 0.1;
                            drag_from = Some((m.column, m.row));
                        }
                    }
                    MouseEventKind::Up(_) => drag_from = None,
                    MouseEventKind::ScrollUp => cam = (cam - 0.3).max(CAM_MIN),
                    MouseEventKind::ScrollDown => cam = (cam + 0.3).min(CAM_MAX),
                    _ => {}
                },
                _ => {}
            }
        }

        let spin = if paused { 0.0 } else { 1.0 };
        ay += dt * (0.8 * spin + vy);
        ax += dt * (0.5 * spin + vx);
        let decay = (-3.0 * dt).exp();
        vx *= decay;
        vy *= decay;

        let (w, h) = (tw as usize, (th as usize).saturating_sub(1).max(1));
        let mut frame = Frame::new(w, h);
        render(&mesh, ax, ay, cam, &mut frame);

        out.clear();
        frame.to_ascii(&mut out);
        out.push_str(&hud(w, paused, fps));

        let mut stdout = io::stdout().lock();
        execute!(stdout, cursor::MoveTo(0, 0))?;
        stdout.write_all(out.as_bytes())?;
        stdout.flush()?;
        drop(stdout);

        if let Some(rest) = frame_time.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

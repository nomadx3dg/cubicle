//! Smoke tests that run the real binary inside a pseudo-terminal: they send keys and
//! SGR mouse taps, and check the screen output, a clean exit, and that the terminal is
//! put back the way it was found.
#![cfg(unix)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

/// A child process attached to a 80x24 pseudo-terminal.
struct Session {
    child: Child,
    master: File,
    /// Kept open so we can inspect the terminal modes the program leaves behind.
    slave: File,
    out: Arc<Mutex<Vec<u8>>>,
    /// This session's private settings file (via CUBICLE_CONFIG).
    config: std::path::PathBuf,
}

static SESSIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Session {
    fn start(args: &[&str]) -> Session {
        Session::start_with_config(args, None)
    }

    /// Like `start`, but first writes `config` to the session's settings file.
    fn start_with_config(args: &[&str], config_text: Option<&str>) -> Session {
        let mut ws = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut m, mut s): (RawFd, RawFd) = (-1, -1);
        // macOS declares the winsize parameter `*mut`, Linux `*const`; `&mut` compiles on both.
        #[allow(clippy::unnecessary_mut_passed)]
        let rc = unsafe {
            libc::openpty(
                &mut m,
                &mut s,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut ws,
            )
        };
        assert_eq!(rc, 0, "openpty failed");
        let master = unsafe { File::from_raw_fd(m) };
        let slave = unsafe { File::from_raw_fd(s) };

        let config = std::env::temp_dir()
            .join(format!(
                "cubicle-pty-{}-{}",
                std::process::id(),
                SESSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ))
            .join("settings.conf");
        if let Some(text) = config_text {
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::write(&config, text).unwrap();
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cubicle"));
        cmd.args(args)
            .env("CUBICLE_CONFIG", &config)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()));
        // Make the pty the child's controlling terminal, like a real login shell would.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = cmd.spawn().expect("failed to launch cubicle");

        let out = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&out);
        let mut reader = master.try_clone().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Session {
            child,
            master,
            slave,
            out,
            config,
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    fn send(&self, bytes: &str) {
        (&self.master).write_all(bytes.as_bytes()).unwrap();
    }

    /// Waits until `needle` appears in the output, or fails with the output's tail.
    fn wait_for(&self, needle: &str) {
        let start = Instant::now();
        while start.elapsed() < WAIT {
            if self.text().contains(needle) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let t = self.text();
        let tail: String = t
            .chars()
            .rev()
            .take(600)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        panic!(
            "timed out waiting for {needle:?}; output ended with:\n{}",
            tail.escape_debug()
        );
    }

    /// True while the terminal is in raw (non-canonical) mode.
    fn is_raw(&self) -> bool {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(self.slave.as_raw_fd(), &mut t) }, 0);
        t.c_lflag & libc::ICANON == 0
    }

    fn wait_exit(&mut self) -> ExitStatus {
        let start = Instant::now();
        while start.elapsed() < WAIT {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("cubicle did not exit; output:\n{}", self.text().escape_debug());
    }

    /// The program has quit: it must exit 0, leave the alternate screen, and restore the tty.
    /// Waits until `needle` has appeared at least `n` times in total.
    fn wait_for_count(&self, needle: &str, n: usize) {
        let start = Instant::now();
        while start.elapsed() < WAIT {
            if self.text().matches(needle).count() >= n {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "{needle:?} never appeared {n} times; screen so far:\n{:?}",
            self.text()
        );
    }

    fn count(&self, needle: &str) -> usize {
        self.text().matches(needle).count()
    }

    /// Waits until any of `needles` has appeared on screen.
    fn wait_for_any(&self, needles: &[&str]) {
        let start = Instant::now();
        while start.elapsed() < WAIT {
            let text = self.text();
            if needles.iter().any(|n| text.contains(n)) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("none of {needles:?} appeared; screen so far:\n{:?}", self.text());
    }

    fn assert_clean_exit(&mut self) {
        let status = self.wait_exit();
        assert!(status.success(), "exit status {status:?}");
        // The process has exited, so its final bytes are already in the pty; the
        // reader thread may just not have collected them yet.
        let start = Instant::now();
        while !self.text().contains("\x1b[?1049l") && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            self.text().contains("\x1b[?1049l"),
            "did not leave the alternate screen"
        );
        assert!(!self.is_raw(), "terminal left in raw mode");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn model_viewer_responds_to_keys_and_quits_cleanly() {
    let mut s = Session::start(&["--colors", "truecolor"]);
    s.wait_for("auto: on");
    assert!(
        s.text().contains("\x1b[?1049h"),
        "did not enter the alternate screen"
    );
    assert!(s.is_raw(), "terminal should be raw while running");
    assert!(
        !s.text().contains("\x1b[38;2;"),
        "plain ASCII mode must not emit colour"
    );

    s.send("m");
    s.wait_for("mode: color");
    s.wait_for("\x1b[38;2;"); // coloured glyphs now appear
    s.send("m");
    s.wait_for("mode: blocks");
    s.wait_for("\u{2580}");

    s.send(" ");
    s.wait_for("auto: off");

    s.send("q");
    s.assert_clean_exit();
}

#[test]
fn escape_also_quits() {
    let mut s = Session::start(&["--cube"]);
    s.wait_for("auto: on");
    s.send("\x1b");
    s.assert_clean_exit();
}

#[test]
fn touching_the_bottom_bar_changes_mode_and_backend() {
    let mut s = Session::start(&["--cube"]);
    s.wait_for("mode: ascii");
    // 80 columns / 5 buttons of 16: [ - ] 0-15, auto 16-31, mode 32-47, backend 48-63, [ + ] 64-79.
    // SGR mouse reports are 1-based: press then release on the last row (24).
    s.send("\x1b[<0;40;24M\x1b[<0;40;24m");
    s.wait_for("mode: color");
    // The backend button cycles cpu -> gpu. Without a usable GPU the frame is still
    // drawn by the CPU and the label shows "cpu*" ("gpu" when a real adapter exists).
    s.send("\x1b[<0;55;24M\x1b[<0;55;24m");
    s.wait_for_any(&["cpu*", "gpu"]);
    s.send("q");
    s.assert_clean_exit();
}

#[test]
fn image_viewer_responds_to_keys_and_quits_cleanly() {
    let mut s = Session::start(&["examples/gradient.png", "--mode", "blocks", "--colors", "256"]);
    s.wait_for("mode: blocks");
    s.wait_for("\u{2580}");
    s.wait_for("48;5;"); // 256-colour background escapes
    assert!(s.is_raw());

    s.send("i");
    s.wait_for("invert: on");

    // Tap the first bar button (mode); with 3 buttons it spans columns 0-25.
    s.send("\x1b[<0;5;24M\x1b[<0;5;24m");
    s.wait_for("mode: ascii");

    s.send("q");
    s.assert_clean_exit();
}

#[test]
fn textured_model_survives_the_texture_toggle() {
    let mut s = Session::start(&[
        "--texture",
        "examples/checker.png",
        "-m",
        "blocks",
        "--colors",
        "truecolor",
    ]);
    s.wait_for("mode: blocks");
    s.wait_for("\u{2580}");
    // 't' switches the texture off and on again; the viewer must keep running.
    s.send("t");
    s.send("t");
    s.send(" ");
    s.wait_for("auto: off");
    s.send("q");
    s.assert_clean_exit();
}

const DOWN: &str = "\x1b[B";
const RIGHT: &str = "\x1b[C";

#[test]
fn bare_launch_opens_the_menu_and_quits_cleanly() {
    let mut s = Session::start(&[]);
    s.wait_for("ASCII 3D in your terminal");
    s.wait_for("Spin the cube");
    s.wait_for("Render settings");
    assert!(s.is_raw());
    s.send("q");
    s.assert_clean_exit();
}

#[test]
fn menu_launches_the_cube_and_returns_to_the_menu() {
    let mut s = Session::start(&[]);
    s.wait_for("Spin the cube");
    let menus = s.count("Spin the cube");
    s.send("1");
    s.wait_for("auto: on"); // the viewer's bottom bar
    s.send("q"); // leaves the viewer, not the program
    s.wait_for_count("Spin the cube", menus + 1);
    s.send("q");
    s.assert_clean_exit();
}

#[test]
fn menu_browser_picks_an_obj_and_remembers_the_folder() {
    let mut s = Session::start(&[]);
    s.wait_for("Browse models");
    s.send("2");
    s.wait_for("Choose a model");
    s.wait_for("torus.obj");
    s.wait_for("sphere.obj");
    // Entries: "..", pyramid.obj, sphere.obj, torus.obj.
    for _ in 0..3 {
        s.send(DOWN);
    }
    s.send("\r");
    s.wait_for("auto: on"); // the model viewer is running
    s.send("q");
    s.wait_for_count("Choose a model", 2); // back in the browser, not the main menu
    let menus = s.count("ASCII 3D in your terminal");
    s.send("\x1b");
    s.wait_for_count("ASCII 3D in your terminal", menus + 1);
    s.send("q");
    s.assert_clean_exit();
    let saved = std::fs::read_to_string(&s.config).expect("settings were saved");
    assert!(
        saved.contains("browse_dir=") && saved.contains("models"),
        "{saved}"
    );
}

#[test]
fn menu_settings_are_saved_and_applied_to_the_viewer() {
    let mut s = Session::start(&[]);
    s.wait_for("Render settings");
    s.send("4");
    s.wait_for("Backend");
    s.wait_for_any(&["GPU unavailable", "GPU: "]); // the probe result is always explained
    s.send(DOWN); // Render mode
    s.send(RIGHT); // ascii -> color
    s.send(DOWN);
    s.send(DOWN);
    s.send(DOWN); // Shading
    s.send(RIGHT); // smooth -> flat
    let menus = s.count("ASCII 3D in your terminal");
    s.send("\x1b"); // back to the main menu
                    // (ESC immediately followed by a key would be read as Alt+key, so wait for the redraw.)
    s.wait_for_count("ASCII 3D in your terminal", menus + 1);
    s.send("1"); // spin the cube with those settings
    s.wait_for("mode: color");
    s.send("q");
    s.send("q");
    s.assert_clean_exit();
    let saved = std::fs::read_to_string(&s.config).expect("settings were saved");
    assert!(
        saved.contains("mode=color") && saved.contains("smooth=false"),
        "{saved}"
    );
}

#[test]
fn saved_settings_are_loaded_on_the_next_launch() {
    let mut s = Session::start_with_config(&[], Some("mode=blocks\nbackend=both\nauto_spin=false\n"));
    s.wait_for("Spin the cube");
    s.wait_for("cpu+gpu"); // summary line shows the saved backend
    s.send("1");
    s.wait_for("mode: blocks");
    s.wait_for("auto: off");
    s.send("q");
    s.send("q");
    s.assert_clean_exit();
}

#[test]
fn a_broken_model_shows_an_error_instead_of_exiting() {
    let dir = std::env::temp_dir().join(format!("cubicle-pty-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.obj"), "v 0 0 0\nf 1 2 3\n").unwrap();
    let cfg = format!("browse_dir={}\n", dir.display());
    let mut s = Session::start_with_config(&[], Some(&cfg));
    s.wait_for("Browse models");
    s.send("2");
    s.wait_for("bad.obj");
    s.send(DOWN);
    s.send("\r");
    s.wait_for("line 2"); // the parser's message, shown in the browser
    let menus = s.count("ASCII 3D in your terminal");
    s.send("\x1b"); // still alive: back to the main menu
    s.wait_for_count("ASCII 3D in your terminal", menus + 1);
    s.send("q");
    s.assert_clean_exit();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn both_backend_runs_live_and_can_be_cycled_with_g() {
    let mut s = Session::start(&["--cube", "--backend", "both", "-m", "blocks"]);
    // "cpu+gpu" with a usable adapter; "cpu*" when the GPU had to be skipped.
    s.wait_for_any(&["cpu+gpu", "cpu*"]);
    s.wait_for("\u{2580}"); // frames are being drawn either way
    s.send("g"); // both -> cpu
    s.wait_for("cpu "); // the HUD label changed (padded with spaces)
    s.send("g"); // cpu -> gpu
    s.send("g"); // gpu -> both
    s.send("q");
    s.assert_clean_exit();
}

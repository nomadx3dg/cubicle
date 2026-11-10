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
}

impl Session {
    fn start(args: &[&str]) -> Session {
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

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cubicle"));
        cmd.args(args)
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
    fn assert_clean_exit(&mut self) {
        let status = self.wait_exit();
        assert!(status.success(), "exit status {status:?}");
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
    let mut s = Session::start(&[]);
    s.wait_for("auto: on");
    s.send("\x1b");
    s.assert_clean_exit();
}

#[test]
fn touching_the_bottom_bar_changes_mode() {
    let mut s = Session::start(&[]);
    s.wait_for("mode: ascii");
    // 80 columns / 4 buttons: [ - ] 0-19, auto 20-39, mode 40-59, [ + ] 60-79.
    // SGR mouse reports are 1-based: press then release on the last row (24).
    s.send("\x1b[<0;50;24M\x1b[<0;50;24m");
    s.wait_for("mode: color");
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

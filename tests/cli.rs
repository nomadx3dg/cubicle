//! End-to-end tests: run the real binary in `--once` mode against the bundled
//! sample assets. No terminal is needed.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cubicle"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to launch cubicle")
}

fn stdout(args: &[&str]) -> String {
    let o = run(args);
    assert!(
        o.status.success(),
        "args {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn ink(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

#[test]
fn version_and_help() {
    assert!(stdout(&["--version"]).starts_with("cubicle "));
    assert!(stdout(&["--help"]).contains("USAGE"));
}

#[test]
fn builtin_cube_renders_in_every_mode() {
    for mode in ["ascii", "color", "blocks"] {
        let out = stdout(&["--once", "--size", "50x16", "--mode", mode]);
        assert_eq!(out.lines().count(), 16, "mode {mode}");
        assert!(ink(&out) > 50, "mode {mode} drew nothing");
        assert_eq!(out.contains('\x1b'), mode != "ascii", "mode {mode}");
    }
}

#[test]
fn bundled_models_render() {
    for model in ["models/sphere.obj", "models/torus.obj", "models/pyramid.obj"] {
        let out = stdout(&[model, "--once", "--size", "50x16", "--rot", "20,30"]);
        assert!(ink(&out) > 50, "{model} drew nothing");
    }
}

#[test]
fn rotation_changes_the_picture_and_is_deterministic() {
    let a = stdout(&["--once", "--size", "50x16", "--rot", "10,10"]);
    let b = stdout(&["--once", "--size", "50x16", "--rot", "10,10"]);
    let c = stdout(&["--once", "--size", "50x16", "--rot", "40,70"]);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn smooth_and_flat_differ_on_curved_models() {
    let smooth = stdout(&["models/sphere.obj", "--once", "--size", "50x16", "--rot", "0,0"]);
    let flat = stdout(&[
        "models/sphere.obj",
        "--once",
        "--size",
        "50x16",
        "--rot",
        "0,0",
        "--flat",
    ]);
    assert_ne!(smooth, flat);
}

#[test]
fn png_renders_and_invert_differs() {
    let normal = stdout(&["examples/gradient.png", "--once", "--size", "60x14"]);
    let inverted = stdout(&["examples/gradient.png", "--once", "--size", "60x14", "--invert"]);
    assert!(ink(&normal) > 100);
    assert_ne!(normal, inverted);
}

#[test]
fn failures_exit_nonzero_with_a_message() {
    let o = run(&["--once", "missing.obj"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot read"));

    let o = run(&["--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown option"));

    let o = run(&["README.md"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn malformed_obj_reports_the_line() {
    let dir = std::env::temp_dir().join("cubicle-test-bad.obj");
    std::fs::write(&dir, "v 0 0 0\nf 1 2 3\n").unwrap();
    let o = run(&["--once", dir.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("line 2"));
    let _ = std::fs::remove_file(dir);
}

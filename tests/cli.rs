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

#[test]
fn colour_depth_controls_the_escape_format() {
    let render =
        |depth: &str, mode: &str| stdout(&["--once", "--size", "50x16", "--mode", mode, "--colors", depth]);
    for mode in ["color", "blocks"] {
        let t = render("truecolor", mode);
        assert!(
            t.contains(";2;") && !t.contains("38;5;") && !t.contains("48;5;"),
            "{mode}"
        );

        let c256 = render("256", mode);
        assert!(c256.contains("38;5;"), "{mode}: 256-colour foreground");
        assert!(!c256.contains(";2;"), "{mode}: no truecolor escapes at 256");

        let c16 = render("16", mode);
        assert!(
            !c16.contains(";2;") && !c16.contains(";5;"),
            "{mode}: only basic colour codes at 16"
        );
        assert!(
            c16.contains("\x1b[3") || c16.contains("\x1b[9"),
            "{mode}: basic fg codes"
        );
    }
}

#[test]
fn plain_ascii_is_unaffected_by_colour_depth() {
    let a = stdout(&["--once", "--size", "40x12", "--colors", "16"]);
    let b = stdout(&["--once", "--size", "40x12", "--colors", "truecolor"]);
    assert_eq!(a, b);
    assert!(!a.contains('\x1b'));
}

#[test]
fn bad_colour_depth_is_a_usage_error() {
    let o = run(&["--colors", "banana"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown colour depth"));
}

#[test]
fn images_honour_colour_depth() {
    let out = stdout(&[
        "examples/gradient.png",
        "--once",
        "--size",
        "40x10",
        "-m",
        "blocks",
        "--colors",
        "256",
    ]);
    assert!(out.contains("48;5;") && !out.contains(";2;"));
}

fn blocks(extra: &[&str]) -> String {
    let mut args = vec![
        "--once",
        "--size",
        "50x16",
        "-m",
        "blocks",
        "--colors",
        "truecolor",
        "--rot",
        "20,30",
    ];
    args.extend_from_slice(extra);
    stdout(&args)
}

#[test]
fn texture_applies_to_models_with_uvs() {
    // The built-in cube and models/sphere.obj carry texture coordinates.
    let tex = "examples/checker.png";
    assert_ne!(blocks(&[]), blocks(&["--texture", tex]));
    assert_ne!(
        blocks(&["models/sphere.obj"]),
        blocks(&["models/sphere.obj", "--texture", tex])
    );
    // The torus has no `vt`, so a texture leaves it unchanged.
    assert_eq!(
        blocks(&["models/torus.obj"]),
        blocks(&["models/torus.obj", "--texture", tex])
    );
}

#[test]
fn texture_errors_are_reported() {
    let o = run(&["--once", "--texture", "nope.png"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot read"));

    let o = run(&["--once", "--texture", "README.md"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("PNG"));

    let o = run(&["--texture", "examples/checker.png", "examples/gradient.png"]);
    assert_eq!(o.status.code(), Some(2));
}

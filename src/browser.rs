//! A small file browser for picking `.obj` models and `.png` images.
//!
//! This is pure state and drawing: no terminal I/O, so it is unit-testable.
//! Directories are listed first, then files whose extension matches the
//! filter. A `..` entry goes up. Hidden (dot) files are off by default.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Parent,
    Dir,
    File,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
}

/// What activating (Enter / tap on the selection) did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Nothing selectable.
    Nothing,
    /// Moved into / out of a directory.
    Navigated,
    /// A file was chosen.
    Chosen(PathBuf),
}

pub struct Browser {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub sel: usize,
    pub top: usize,
    pub show_hidden: bool,
    /// Lower-case extensions to show, without the dot.
    exts: Vec<&'static str>,
    /// Set when the directory could not be read.
    pub error: Option<String>,
}

pub const HEADER_ROWS: usize = 2;
pub const FOOTER_ROWS: usize = 1;

fn matches(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn human_size(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// Pads or truncates to exactly `w` characters (truncation keeps the start).
pub fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        s.chars().take(w).collect()
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

/// Shortens a long path to `w` characters by eliding the beginning.
pub fn tail(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w || w < 2 {
        return s.chars().take(w).collect();
    }
    format!("…{}", s.chars().skip(n - (w - 1)).collect::<String>())
}

impl Browser {
    /// Starts in `start` (or its parent if it is a file, or `.` if it does not exist).
    pub fn new(start: &Path, exts: &[&'static str]) -> Browser {
        let mut dir = if start.is_dir() {
            start.to_path_buf()
        } else {
            start.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        if !dir.is_dir() {
            dir = PathBuf::from(".");
        }
        let dir = dir.canonicalize().unwrap_or(dir);
        let mut b = Browser {
            dir,
            entries: Vec::new(),
            sel: 0,
            top: 0,
            show_hidden: false,
            exts: exts.to_vec(),
            error: None,
        };
        b.refresh();
        b
    }

    /// Re-reads the current directory.
    pub fn refresh(&mut self) {
        self.error = None;
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }
                    let path = e.path();
                    // metadata() follows symlinks; broken links are skipped.
                    let Ok(meta) = std::fs::metadata(&path) else {
                        continue;
                    };
                    if meta.is_dir() {
                        dirs.push(Entry {
                            name,
                            path,
                            kind: Kind::Dir,
                            size: 0,
                        });
                    } else if matches(&path, &self.exts) {
                        files.push(Entry {
                            name,
                            path,
                            kind: Kind::File,
                            size: meta.len(),
                        });
                    }
                }
            }
            Err(e) => self.error = Some(format!("cannot read folder: {e}")),
        }
        let key = |e: &Entry| e.name.to_lowercase();
        dirs.sort_by_key(key);
        files.sort_by_key(key);
        self.entries.clear();
        if let Some(parent) = self.dir.parent() {
            self.entries.push(Entry {
                name: "..".into(),
                path: parent.to_path_buf(),
                kind: Kind::Parent,
                size: 0,
            });
        }
        self.entries.extend(dirs);
        self.entries.extend(files);
        self.sel = self.sel.min(self.entries.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.sel)
    }

    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| e.kind == Kind::File).count()
    }

    pub fn move_sel(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.sel = (self.sel as isize + delta).clamp(0, last) as usize;
    }

    pub fn home(&mut self) {
        self.sel = 0;
    }

    pub fn end(&mut self) {
        self.sel = self.entries.len().saturating_sub(1);
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh();
    }

    fn enter(&mut self, dir: PathBuf, select: Option<&str>) {
        self.dir = dir.canonicalize().unwrap_or(dir);
        self.sel = 0;
        self.top = 0;
        self.refresh();
        if let Some(name) = select {
            if let Some(i) = self.entries.iter().position(|e| e.name == name) {
                self.sel = i;
            }
        }
    }

    /// Goes to the parent folder, re-selecting the one we came from.
    pub fn go_up(&mut self) -> bool {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else {
            return false;
        };
        let from = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
        self.enter(parent, from.as_deref());
        true
    }

    /// Opens the selected folder or chooses the selected file.
    pub fn activate(&mut self) -> Pick {
        let Some(e) = self.selected().cloned() else {
            return Pick::Nothing;
        };
        match e.kind {
            Kind::Parent => {
                self.go_up();
                Pick::Navigated
            }
            Kind::Dir => {
                self.enter(e.path, None);
                Pick::Navigated
            }
            Kind::File => Pick::Chosen(e.path),
        }
    }

    /// Rows available for the list in a terminal `h` rows tall.
    pub fn visible_rows(h: usize) -> usize {
        h.saturating_sub(HEADER_ROWS + FOOTER_ROWS).max(1)
    }

    /// Scrolls so the selection is on screen.
    pub fn ensure_visible(&mut self, h: usize) {
        let v = Self::visible_rows(h);
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + v {
            self.top = self.sel + 1 - v;
        }
        self.top = self.top.min(self.entries.len().saturating_sub(v));
    }

    /// A tap on terminal row `row`. The first tap selects; tapping the
    /// already-selected entry activates it (so a stray tap never opens a file).
    pub fn tap(&mut self, row: usize, h: usize) -> Pick {
        if row < HEADER_ROWS || row >= HEADER_ROWS + Self::visible_rows(h) {
            return Pick::Nothing;
        }
        let idx = self.top + (row - HEADER_ROWS);
        if idx >= self.entries.len() {
            return Pick::Nothing;
        }
        if idx == self.sel {
            self.activate()
        } else {
            self.sel = idx;
            Pick::Nothing
        }
    }

    /// Draws the browser into `h` rows of `w` columns.
    pub fn draw(&mut self, title: &str, w: usize, h: usize) -> Vec<String> {
        self.ensure_visible(h);
        let mut rows = Vec::with_capacity(h);
        rows.push(format!("\x1b[1m{}\x1b[0m", fit(&format!(" {title}"), w)));
        rows.push(format!(
            "\x1b[2m{}\x1b[0m",
            fit(
                &format!(" {}", tail(&self.dir.to_string_lossy(), w.saturating_sub(1))),
                w
            )
        ));
        let v = Self::visible_rows(h);
        for i in 0..v {
            let idx = self.top + i;
            let line = match self.entries.get(idx) {
                Some(e) => {
                    let (icon, extra) = match e.kind {
                        Kind::Parent => ("↰ ", String::new()),
                        Kind::Dir => ("▸ ", String::new()),
                        Kind::File => ("  ", human_size(e.size)),
                    };
                    let name = if e.kind == Kind::Dir {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    };
                    let size_w = extra.chars().count();
                    let name_w = w.saturating_sub(3 + size_w + 1);
                    let left = fit(
                        &format!(" {icon}{}", fit(&name, name_w)),
                        w.saturating_sub(size_w + 1),
                    );
                    format!("{left}{extra} ")
                }
                None => String::new(),
            };
            let line = fit(&line, w);
            if idx == self.sel && !self.entries.is_empty() {
                rows.push(format!("\x1b[7m{line}\x1b[0m"));
            } else {
                rows.push(line);
            }
        }
        // Explain an empty or unreadable folder inside the list area.
        let note = match (&self.error, self.file_count()) {
            (Some(e), _) => Some(e.clone()),
            (None, 0) if self.entries.len() <= 1 && v > 1 => Some("no matching files here".to_string()),
            _ => None,
        };
        if let (Some(n), true) = (note, v > 2) {
            rows[HEADER_ROWS + 1] = fit(&format!("   {n}"), w);
        }
        let help = if w >= 56 {
            " ↑↓ move · Enter open · ⌫ up · h hidden · Esc back"
        } else {
            " ↑↓ · Enter · ⌫ up · Esc"
        };
        rows.push(format!("\x1b[2m{}\x1b[0m", fit(help, w)));
        rows.truncate(h);
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Builds a scratch tree:
    /// root/{a_dir/{inner.obj}, B_dir/, .hidden.obj, z.obj, Cube.OBJ, pic.png, notes.txt}
    fn tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cubicle-browser-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a_dir")).unwrap();
        fs::create_dir_all(root.join("B_dir")).unwrap();
        for f in [
            "a_dir/inner.obj",
            ".hidden.obj",
            "z.obj",
            "Cube.OBJ",
            "pic.png",
            "notes.txt",
        ] {
            fs::write(root.join(f), "v 0 0 0\n").unwrap();
        }
        root.canonicalize().unwrap()
    }

    fn names(b: &Browser) -> Vec<String> {
        b.entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn lists_dirs_first_then_matching_files_case_insensitively() {
        let root = tree("list");
        let b = Browser::new(&root, &["obj"]);
        // Parent, dirs (sorted ignoring case), then .obj files (no .png / .txt / hidden).
        assert_eq!(names(&b), ["..", "a_dir", "B_dir", "Cube.OBJ", "z.obj"]);
        assert_eq!(b.file_count(), 2);
        let b = Browser::new(&root, &["png"]);
        assert_eq!(names(&b), ["..", "a_dir", "B_dir", "pic.png"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hidden_files_toggle() {
        let root = tree("hidden");
        let mut b = Browser::new(&root, &["obj"]);
        assert!(!names(&b).contains(&".hidden.obj".to_string()));
        b.toggle_hidden();
        assert!(names(&b).contains(&".hidden.obj".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn navigation_enter_dir_and_back_up_reselects() {
        let root = tree("nav");
        let mut b = Browser::new(&root, &["obj"]);
        b.sel = names(&b).iter().position(|n| n == "a_dir").unwrap();
        assert_eq!(b.activate(), Pick::Navigated);
        assert_eq!(b.dir, root.join("a_dir"));
        assert_eq!(names(&b), ["..", "inner.obj"]);
        // Backspace/up returns and selects the folder we left.
        assert!(b.go_up());
        assert_eq!(b.dir, root);
        assert_eq!(b.selected().unwrap().name, "a_dir");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn choosing_a_file_returns_its_path() {
        let root = tree("pick");
        let mut b = Browser::new(&root, &["obj"]);
        b.sel = names(&b).iter().position(|n| n == "z.obj").unwrap();
        assert_eq!(b.activate(), Pick::Chosen(root.join("z.obj")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selection_is_clamped_and_scrolls() {
        let root = tree("scroll");
        for i in 0..30 {
            fs::write(root.join(format!("m{i:02}.obj")), "v 0 0 0\n").unwrap();
        }
        let mut b = Browser::new(&root, &["obj"]);
        b.move_sel(-5);
        assert_eq!(b.sel, 0);
        b.end();
        assert_eq!(b.sel, b.entries.len() - 1);
        b.move_sel(10);
        assert_eq!(b.sel, b.entries.len() - 1);
        b.ensure_visible(10); // 7 visible rows
        assert!(b.sel >= b.top && b.sel < b.top + Browser::visible_rows(10));
        b.home();
        b.ensure_visible(10);
        assert_eq!(b.top, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tap_selects_then_activates() {
        let root = tree("tap");
        let mut b = Browser::new(&root, &["obj"]);
        let h = 20;
        let z = b.entries.iter().position(|e| e.name == "z.obj").unwrap();
        let row = HEADER_ROWS + z;
        assert_eq!(b.tap(row, h), Pick::Nothing, "first tap only selects");
        assert_eq!(b.sel, z);
        assert_eq!(b.tap(row, h), Pick::Chosen(root.join("z.obj")));
        assert_eq!(b.tap(0, h), Pick::Nothing, "header taps do nothing");
        assert_eq!(
            b.tap(HEADER_ROWS + 15, h),
            Pick::Nothing,
            "below the list does nothing"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_missing_and_unreadable_start_dirs_do_not_panic() {
        let b = Browser::new(Path::new("/definitely/not/here"), &["obj"]);
        assert!(b.dir.is_dir());
        let root = tree("empty");
        let empty = root.join("B_dir");
        let mut b = Browser::new(&empty, &["obj"]);
        assert_eq!(names(&b), [".."]);
        assert_eq!(b.activate(), Pick::Navigated); // ".." goes up
                                                   // Starting at a file starts in its folder.
        let b = Browser::new(&root.join("z.obj"), &["obj"]);
        assert_eq!(b.dir, root);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn drawing_fits_the_screen_and_highlights_the_selection() {
        let root = tree("draw");
        let mut b = Browser::new(&root, &["obj"]);
        for (w, h) in [(80, 24), (30, 8), (12, 4), (1, 1)] {
            let rows = b.draw("Pick a model", w, h);
            assert!(rows.len() <= h.max(1), "{w}x{h}");
            for r in &rows {
                // Strip escapes, then check the visible width.
                let plain: String = {
                    let mut out = String::new();
                    let mut chars = r.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '\x1b' {
                            for d in chars.by_ref() {
                                if d == 'm' {
                                    break;
                                }
                            }
                        } else {
                            out.push(c);
                        }
                    }
                    out
                };
                assert!(plain.chars().count() <= w, "row too wide at {w}x{h}: {plain:?}");
            }
        }
        let rows = b.draw("Pick a model", 60, 12);
        assert!(
            rows.iter().any(|r| r.contains("\x1b[7m")),
            "selection is reverse-video"
        );
        assert!(rows.iter().any(|r| r.contains("Cube.OBJ")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn text_helpers() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1_048_576), "3.0 MB");
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 3), "abc");
        assert_eq!(tail("/a/very/long/path", 8), "…ng/path");
        assert_eq!(tail("short", 8), "short");
    }
}

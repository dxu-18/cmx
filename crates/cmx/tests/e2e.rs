//! End-to-end tests: run the real `cmx` binary in a pseudo-terminal, use our own cmx-vt emulator
//! as the "host terminal", send keystrokes / mouse events, and assert on the rendered screen and
//! on files written to disk.

use cmx_sys::poll::{POLLIN, PollFd, poll};
use cmx_sys::pty::{CommandSpec, Pty};
use cmx_vt::Term;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("cmx-e2e-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(std::fs::canonicalize(&p).unwrap())
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    pty: Pty,
    vt: Term,
    kitty: bool,
    raw: Vec<u8>,
    exited: bool,
}

impl Harness {
    fn start(args: &[&str], cwd: &Path, kitty: bool, env: &[(&str, &str)]) -> Harness {
        let mut spec = CommandSpec::new(env!("CARGO_BIN_EXE_cmx")).cwd(cwd);
        for a in args {
            spec = spec.arg(*a);
        }
        spec = spec
            .env("TERM", "xterm-256color")
            .env("SHELL", "/bin/sh")
            .env("HOME", cwd.to_string_lossy().to_string())
            .env("ENV", "")
            .env("PS1", "$ ")
            .env("TERM_PROGRAM", "cmx-e2e");
        // the harness simulates the terminal; the real keyboard of this machine must not leak in
        if !env.iter().any(|(k, _)| *k == "CMX_MODPROBE") {
            spec = spec.env("CMX_MODPROBE", "0");
        }
        for (k, v) in env {
            spec = spec.env(*k, *v);
        }
        let pty = Pty::spawn(&spec, 100, 30, None).expect("spawn cmx");
        Harness { pty, vt: Term::new(100, 30), kitty, raw: Vec::new(), exited: false }
    }

    fn write(&mut self, mut b: &[u8]) {
        while !b.is_empty() {
            match self.pty.write(b) {
                Ok(n) => b = &b[n..],
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                Err(e) => panic!("write: {e}"),
            }
        }
    }

    /// Read output for up to `dur`, feeding the emulator and answering terminal queries.
    fn pump(&mut self, dur: Duration) {
        let end = Instant::now() + dur;
        let mut buf = [0u8; 65536];
        while Instant::now() < end && !self.exited {
            let left = end.saturating_duration_since(Instant::now()).as_millis().max(1) as i32;
            let mut fds = [PollFd::new(self.pty.fd(), POLLIN)];
            poll(&mut fds, left.min(20)).unwrap();
            loop {
                match self.pty.read(&mut buf) {
                    Ok(0) => {
                        self.exited = true;
                        break;
                    }
                    Ok(n) => {
                        let chunk = &buf[..n];
                        if self.kitty && chunk.windows(4).any(|w| w == b"\x1b[?u") {
                            self.write(b"\x1b[?1u");
                        }
                        self.raw.extend_from_slice(chunk);
                        self.vt.feed(chunk);
                        if !self.vt.responses.is_empty() {
                            let r = std::mem::take(&mut self.vt.responses);
                            self.write(&r);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        self.exited = true;
                        break;
                    }
                }
            }
        }
    }

    fn send(&mut self, b: &[u8]) {
        self.write(b);
        self.pump(Duration::from_millis(40));
    }

    /// Send a lone ESC and wait past the ESC-disambiguation timeout.
    fn esc(&mut self) {
        self.write(b"\x1b");
        self.pump(Duration::from_millis(80));
    }

    fn screen(&self) -> String {
        let mut s = String::new();
        for row in self.vt.grid() {
            for c in &row.cells {
                if !c.sym.is_cont() {
                    s.push_str(c.sym.as_str());
                }
            }
            s.push('\n');
        }
        s
    }

    fn row(&self, y: usize) -> String {
        let mut s = String::new();
        for c in &self.vt.grid()[y].cells {
            if !c.sym.is_cont() {
                s.push_str(c.sym.as_str());
            }
        }
        s
    }

    fn wait_for(&mut self, needle: &str, secs: u64) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if self.screen().contains(needle) {
                return;
            }
            self.pump(Duration::from_millis(30));
        }
        panic!("timed out waiting for {needle:?}; screen:\n{}", self.screen());
    }

    fn wait_exit(&mut self) -> i32 {
        let end = Instant::now() + Duration::from_secs(5);
        while !self.exited && Instant::now() < end {
            self.pump(Duration::from_millis(30));
        }
        for _ in 0..200 {
            if let Some(c) = self.pty.try_wait() {
                return c;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("cmx did not exit; screen:\n{}", self.screen());
    }

    fn click(&mut self, x: u16, y: u16) {
        let s = format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1);
        self.send(s.as_bytes());
    }
}

#[test]
fn edit_type_save_quit() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit", "a.txt"], d.path(), false, &[]);
    h.wait_for("a.txt", 5);
    h.send(b"hello world");
    h.wait_for("hello world", 2);
    h.send(b"\x13"); // Ctrl+S
    h.wait_for("Saved a.txt", 2);
    h.send(b"\x11"); // Ctrl+Q
    assert_eq!(h.wait_exit(), 0);
    assert_eq!(std::fs::read_to_string(d.file("a.txt")).unwrap(), "hello world");
}

#[test]
fn untitled_save_as_and_newlines() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("Untitled-1", 5);
    h.send(b"fn main() {");
    h.send(b"\r");
    h.send(b"x");
    h.send(b"\x13");
    h.wait_for("Save as", 2);
    h.send(b"src/b.rs\r");
    h.wait_for("Saved b.rs", 2);
    h.send(b"\x11");
    h.wait_exit();
    // auto-close bracket + auto-indent between braces
    assert_eq!(std::fs::read_to_string(d.file("src/b.rs")).unwrap(), "fn main() {\n    x\n}");
}

#[test]
fn undo_redo_legacy_keys() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit", "u.txt"], d.path(), false, &[]);
    h.wait_for("u.txt", 5);
    h.send(b"one two");
    h.send(b"\x1a"); // Ctrl+Z
    h.wait_for("one", 1);
    h.send(b"\x1bOS"); // F4 = redo
    h.wait_for("one two", 1);
    h.send(b"\x1a");
    h.send(b"\x13");
    h.wait_for("Saved", 2);
    h.send(b"\x11");
    h.wait_exit();
    assert_eq!(std::fs::read_to_string(d.file("u.txt")).unwrap(), "one");
}

#[test]
fn vim_mode_insert_and_ex() {
    let d = TempDir::new();
    std::fs::write(d.file("v.txt"), "foo bar foo\nsecond line\n").unwrap();
    let mut h = Harness::start(&["edit", "v.txt"], d.path(), false, &[]);
    h.wait_for("foo bar foo", 5);
    h.send(b"\x0b"); // Ctrl+K
    h.wait_for("VIM NORMAL", 2);
    h.send(b"jdd");
    h.send(b"ggAX");
    h.esc();
    h.wait_for("VIM NORMAL", 2);
    h.send(b":%s/foo/baz/g\r");
    h.wait_for("2 substitutions", 2);
    h.send(b":wq\r");
    h.wait_exit();
    assert_eq!(std::fs::read_to_string(d.file("v.txt")).unwrap(), "baz bar bazX\n");
}

#[test]
fn vim_wq_on_untitled_asks_name_then_quits() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("Untitled-1", 5);
    h.send(b"\x0b");
    h.send(b"ihi");
    h.esc();
    h.send(b":wq\r");
    h.wait_for("Save as", 2);
    h.send(b"named.txt\r");
    assert_eq!(h.wait_exit(), 0);
    assert_eq!(std::fs::read_to_string(d.file("named.txt")).unwrap(), "hi");
}

#[test]
fn quit_confirmation_when_dirty() {
    let d = TempDir::new();
    std::fs::write(d.file("q.txt"), "keep").unwrap();
    let mut h = Harness::start(&["edit", "q.txt"], d.path(), false, &[]);
    h.wait_for("keep", 5);
    h.send(b"zz");
    h.send(b"\x11");
    h.wait_for("Quit anyway?", 2);
    h.send(b"n");
    assert!(!h.exited);
    h.send(b"\x11");
    h.wait_for("Quit anyway?", 2);
    h.send(b"y");
    h.wait_exit();
    assert_eq!(std::fs::read_to_string(d.file("q.txt")).unwrap(), "keep");
}

#[test]
fn mouse_click_double_click_and_type() {
    let d = TempDir::new();
    std::fs::write(d.file("m.txt"), "hello world").unwrap();
    let mut h = Harness::start(&["edit", "m.txt"], d.path(), false, &[]);
    h.wait_for("hello world", 5);
    // gutter is 5 columns ("   1 " + pad): text starts at x=5 on row 1
    let wx = 5 + 6;
    h.click(wx, 1);
    h.click(wx, 1); // double click → select "world"
    h.wait_for("(5 selected)", 2);
    h.send(b"there");
    h.wait_for("hello there", 2);
    // drag-select "hello" and delete it
    h.send(format!("\x1b[<0;{};2M\x1b[<32;{};2M\x1b[<0;{};2m", 6, 11, 11).as_bytes());
    h.wait_for("(5 selected)", 2);
    h.send(b"\x7f");
    h.send(b"\x13");
    h.wait_for("Saved", 2);
    h.send(b"\x11");
    h.wait_exit();
    assert_eq!(std::fs::read_to_string(d.file("m.txt")).unwrap(), " there");
}

#[test]
fn legacy_ctrl_l_toggles_explorer() {
    let d = TempDir::new();
    std::fs::write(d.file("visible_file.txt"), "x").unwrap();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("⌨ legacy", 5);
    h.send(b"\x0c"); // Ctrl+L == Ctrl+Shift+L on a legacy terminal
    h.wait_for("EXPLORER", 2);
    h.wait_for("visible_file.txt", 2);
    // explorer focused: Enter opens the selected file
    h.send(b"\r");
    h.wait_for("Ln 1, Col 1", 2);
    assert!(h.screen().contains("visible_file.txt"));
    h.send(b"\x11");
    h.wait_exit();
}

#[test]
fn kitty_protocol_ctrl_shift_chords() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), true, &[]);
    h.wait_for("⌨ kitty", 5);
    assert!(h.raw.windows(5).any(|w| w == b"\x1b[>1u"), "kitty flags were not pushed");
    h.send(b"\x1b[108;6u"); // Ctrl+Shift+L
    h.wait_for("EXPLORER", 2);
    h.send(b"\x1b[108;6u"); // explorer focused → hide it
    h.pump(Duration::from_millis(100));
    assert!(!h.screen().contains("EXPLORER"));
    h.send(b"\x1b[113;5u"); // Ctrl+Q
    h.wait_exit();
}

/// With an enhanced keyboard protocol Ctrl+Shift+Z (redo) and Ctrl+Z (undo) are distinct, and Cmd
/// chords are left alone (EDITOR-DESIGN D-16).
#[test]
fn kitty_ctrl_shift_z_redo_and_cmd_ignored() {
    let d = TempDir::new();
    std::fs::write(d.file("r.txt"), "").unwrap();
    let mut h = Harness::start(&["edit", "r.txt"], d.path(), true, &[]);
    h.wait_for("⌨ kitty", 5);
    h.send(b"ab");
    h.wait_for("ab", 2);
    h.send(b"\x1b[122;5u"); // Ctrl+Z
    h.pump(Duration::from_millis(100));
    assert!(!h.row(1).contains("ab"), "undo:\n{}", h.screen());
    h.send(b"\x1b[122;6u"); // Ctrl+Shift+Z
    h.wait_for("ab", 2);
    h.send(b"\x1b[115;9u"); // Cmd+S: not a cmx command
    h.pump(Duration::from_millis(150));
    assert_eq!(std::fs::read_to_string(d.file("r.txt")).unwrap(), "", "Cmd+S must not save");
    h.send(b"\x1b[115;5u"); // Ctrl+S
    let t0 = Instant::now();
    while std::fs::read_to_string(d.file("r.txt")).unwrap() != "ab" {
        assert!(t0.elapsed() < Duration::from_secs(3), "Ctrl+S did not save");
        h.pump(Duration::from_millis(30));
    }
    h.send(b"\x1b[113;5u"); // Ctrl+Q
    h.wait_exit();
}

#[test]
fn workspace_explorer_preview_and_terminal() {
    let d = TempDir::new();
    std::fs::write(d.file("alpha.txt"), "ALPHA_CONTENT").unwrap();
    std::fs::write(d.file("beta.rs"), "fn beta() {}").unwrap();
    std::fs::create_dir(d.file("sub")).unwrap();
    let mut h = Harness::start(&["workspace"], d.path(), false, &[]);
    h.wait_for("EXPLORER", 5);
    h.wait_for("Open a file to begin", 5);
    h.wait_for("TERMINAL", 5);
    // explorer has focus: Down moves past "sub" to alpha.txt and previews it
    h.send(b"\x1b[B");
    h.wait_for("ALPHA_CONTENT", 3);
    h.send(b"\x1b[B");
    h.wait_for("fn beta() {}", 3);
    assert!(!h.screen().contains("ALPHA_CONTENT"), "preview tab should be replaced");
    // F6 cycles focus: explorer → editor → terminal
    h.send(b"\x1b[17~");
    h.send(b"\x1b[17~");
    h.wait_for("TERMINAL sh", 3);
    h.send(b"echo cmx_$((40+2))\r");
    h.wait_for("cmx_42", 5);
    // the shell keeps Ctrl+L (clear) on a legacy terminal. Shells differ in what they do with it:
    // bash clears, dash (Ubuntu's sh) inserts ^L, so finish the line before typing `exit`.
    h.send(b"\x0c");
    h.pump(Duration::from_millis(300));
    h.send(b"\r");
    h.pump(Duration::from_millis(200));
    assert!(h.screen().contains("TERMINAL"));
    h.send(b"exit\r");
    h.pump(Duration::from_millis(500));
    // the panel closes when its last shell exits; focus returns to the editor
    h.wait_for("Ln 1, Col 1", 3);
    h.send(b"\x11");
    h.wait_exit();
}

#[test]
fn drag_explorer_border_resizes() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("⌨ legacy", 5);
    h.send(b"\x02"); // Ctrl+B: explorer
    h.wait_for("EXPLORER", 2);
    // the border lights up (┃) while hovered or dragged
    let border_x = |h: &Harness| h.row(5).chars().position(|c| c == '│' || c == '┃');
    let before = border_x(&h).expect("border visible");
    let (x0, x1) = (before as u16 + 1, before as u16 + 16);
    h.send(format!("\x1b[<0;{x0};6M\x1b[<32;{x1};6M\x1b[<0;{x1};6m").as_bytes());
    h.pump(Duration::from_millis(100));
    let after = border_x(&h).expect("border visible");
    assert_eq!(after, before + 15, "border should follow the drag:\n{}", h.screen());
    h.send(b"\x11");
    h.wait_exit();
}

fn row_of(h: &Harness, needle: &str) -> Option<usize> {
    (0..30).find(|&y| h.row(y).contains(needle))
}

fn wait_file(h: &mut Harness, path: &Path, want: &str) {
    let t0 = Instant::now();
    while std::fs::read_to_string(path).unwrap_or_default() != want {
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "file {} = {:?}, want {want:?}; screen:\n{}",
            path.display(),
            std::fs::read_to_string(path).unwrap_or_default(),
            h.screen()
        );
        h.pump(Duration::from_millis(30));
    }
}

/// Prose files open with soft wrap; Alt+Z toggles it.
#[test]
fn soft_wrap_long_lines() {
    let d = TempDir::new();
    let long = "word ".repeat(40);
    std::fs::write(d.file("w.md"), format!("{long}END\nsecond line")).unwrap();
    let mut h = Harness::start(&["edit", "w.md"], d.path(), false, &[]);
    h.wait_for("w.md", 5);
    h.wait_for("END", 3);
    let second = row_of(&h, "second line").expect("line 2 visible");
    assert!(second >= 3, "line 1 should take 3 rows:\n{}", h.screen());
    // continuation rows have no line number
    assert!(h.row(2).trim_start().starts_with("word"), "{}", h.screen());
    h.send(b"\x1bz"); // Alt+Z
    h.wait_for("Word wrap off", 2);
    assert!(!h.screen().contains("END"), "no wrap: the end of line 1 is off screen\n{}", h.screen());
    assert_eq!(row_of(&h, "second line"), Some(2));
    // End on line 1 scrolls horizontally to it
    h.send(b"\x1b[F");
    h.wait_for("END", 2);
    h.send(b"\x11");
    h.wait_exit();
}

/// Ctrl+H, Alt+R: a regex replacement with capture groups.
#[test]
fn regex_find_and_replace_with_groups() {
    let d = TempDir::new();
    std::fs::write(d.file("r.txt"), "a1 b22 c333 x\n").unwrap();
    let mut h = Harness::start(&["edit", "r.txt"], d.path(), true, &[]);
    h.wait_for("⌨ kitty", 5);
    h.send(b"\x1b[102;5u"); // Ctrl+F
    h.wait_for("Find", 2);
    h.send(b"\x1b[114;3u"); // Alt+R
    h.send(br"\d+");
    h.wait_for("Find [3] [.*]", 2);
    h.esc();
    h.send(b"\x1b[104;5u"); // Ctrl+H
    h.wait_for("Replace", 2);
    h.send(br"([a-z])(\d+)");
    h.wait_for("Replace [3] [.*]", 2);
    h.send(b"\r");
    h.wait_for("with:", 2);
    h.send(b"$2$1\r");
    h.wait_for("Replace this match?", 2);
    h.send(b"a");
    h.wait_for("Replaced 3 occurrences", 2);
    h.send(b"\x1b[115;5u"); // Ctrl+S
    wait_file(&mut h, &d.file("r.txt"), "1a 22b 333c x\n");
    // an invalid pattern is reported, not crashed on
    h.send(b"\x1b[102;5u");
    h.send(b"(");
    h.wait_for("invalid regex", 2);
    h.esc();
    h.send(b"\x1b[113;5u");
    h.wait_exit();
}

/// Ctrl+D adds the next occurrence; typing edits every cursor; one undo step.
#[test]
fn multiple_cursors() {
    let d = TempDir::new();
    std::fs::write(d.file("m.txt"), "foo = 1\nfoo = 2\nfoo = 3\n").unwrap();
    let mut h = Harness::start(&["edit", "m.txt"], d.path(), true, &[]);
    h.wait_for("⌨ kitty", 5);
    for _ in 0..3 {
        h.send(b"\x1b[100;5u"); // Ctrl+D
    }
    h.send(b"bar");
    h.wait_for("bar = 3", 2);
    h.send(b"\x1b[115;5u");
    wait_file(&mut h, &d.file("m.txt"), "bar = 1\nbar = 2\nbar = 3\n");
    h.send(b"\x1b[122;5u"); // Ctrl+Z: all three at once
    h.wait_for("foo = 3", 2);
    assert!(!h.screen().contains("bar ="), "{}", h.screen());
    // Esc → one cursor; Ctrl+Alt+Down twice adds cursors below; End + text appends to each line
    h.send(b"\x1b[27u");
    h.send(b"\x1b[1;5H"); // Ctrl+Home
    h.send(b"\x1b[1;7B"); // Ctrl+Alt+Down
    h.send(b"\x1b[1;7B");
    h.send(b"\x1b[F"); // End
    h.send(b";");
    h.wait_for("foo = 3;", 2);
    // a plain click → one cursor; Alt+click adds another (text starts at column 5, line 1 = row 1)
    h.send(b"\x1b[<0;6;2M\x1b[<0;6;2m");
    h.send(b"\x1b[<8;6;4M\x1b[<8;6;4m");
    h.send(b"#");
    h.send(b"\x1b[115;5u");
    wait_file(&mut h, &d.file("m.txt"), "#foo = 1;\nfoo = 2;\n#foo = 3;\n");
    h.send(b"\x1b[113;5u");
    h.wait_exit();
}

/// External changes: a clean buffer reloads, the explorer picks up new files, an edited buffer is
/// flagged instead of overwritten.
#[test]
fn files_changed_on_disk() {
    let d = TempDir::new();
    std::fs::write(d.file("w.txt"), "ORIGINAL\n").unwrap();
    let mut h = Harness::start(&["edit", "w.txt"], d.path(), false, &[]);
    h.wait_for("ORIGINAL", 5);
    h.send(b"\x02"); // Ctrl+B: explorer
    h.wait_for("EXPLORER", 2);
    std::fs::write(d.file("w.txt"), "CHANGED_OUTSIDE\n").unwrap();
    h.wait_for("CHANGED_OUTSIDE", 4);
    h.wait_for("Reloaded w.txt", 2);
    std::fs::write(d.file("brand_new.txt"), "").unwrap();
    h.wait_for("brand_new.txt", 4);
    std::fs::remove_file(d.file("brand_new.txt")).unwrap();
    let t0 = Instant::now();
    while h.screen().contains("brand_new.txt") {
        assert!(t0.elapsed() < Duration::from_secs(4), "deleted file still listed:\n{}", h.screen());
        h.pump(Duration::from_millis(50));
    }
    // edit, then change on disk: flagged, not reloaded
    h.click(50, 3);
    h.send(b"x");
    std::fs::write(d.file("w.txt"), "THIRD\n").unwrap();
    h.wait_for("changed on disk", 4);
    assert!(h.screen().contains("w.txt ⚠"), "{}", h.screen());
    assert!(!h.screen().contains("THIRD"));
    h.send(b"\x13"); // Ctrl+S asks before overwriting
    h.wait_for("Overwrite it?", 2);
    h.send(b"y");
    h.wait_for("Saved w.txt", 2);
    assert!(std::fs::read_to_string(d.file("w.txt")).unwrap().contains('x'));
    h.send(b"\x11");
    h.wait_exit();
}

/// Hovering a splitter highlights it and asks the terminal for a resize pointer (OSC 22).
#[test]
fn splitter_hover_feedback() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("⌨ legacy", 5);
    h.send(b"\x02");
    h.wait_for("EXPLORER", 2);
    let bx = h.row(5).chars().position(|c| c == '│').expect("border") as u16;
    h.send(format!("\x1b[<35;{};6M", bx + 1).as_bytes()); // motion, no button
    h.pump(Duration::from_millis(60));
    // a ↔ resize marker under the pointer (any terminal), the rest of the border lit up
    assert_eq!(h.row(5).chars().nth(bx as usize), Some('↔'), "{}", h.screen());
    assert_eq!(h.row(6).chars().nth(bx as usize), Some('┃'), "{}", h.screen());
    assert!(String::from_utf8_lossy(&h.raw).contains("\x1b]22;ew-resize"));
    h.send(b"\x1b[<35;60;6M");
    h.pump(Duration::from_millis(60));
    assert_eq!(h.row(5).chars().nth(bx as usize), Some('│'));
    assert!(String::from_utf8_lossy(&h.raw).contains("\x1b]22;text"));
    h.send(b"\x11");
    h.wait_exit();
}

/// With the OS Shift probe on but unusable (macOS without Input Monitoring, or no X11), legacy
/// Ctrl+L/Ctrl+Y must still open the panels (regression: the probe used to report "no Shift").
#[test]
fn legacy_panels_work_with_probe_enabled() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[("CMX_MODPROBE", "1")]);
    h.wait_for("⌨ legacy", 5);
    if h.screen().contains("legacy+⇧") {
        // the probe really works here (X11 / Input Monitoring granted): Ctrl+L is select-line
        h.send(b"\x11");
        h.wait_exit();
        return;
    }
    h.send(b"\x0c"); // Ctrl+L
    h.wait_for("EXPLORER", 2);
    h.send(b"\x19"); // Ctrl+Y
    h.wait_for("TERMINAL", 3);
    h.send(b"\x1b[17~"); // F6 out of the terminal
    h.send(b"\x11");
    h.wait_exit();
}

/// Ctrl+arrows go to line / document ends, Ctrl+Backspace / Ctrl+Delete delete to line start /
/// end, Alt+Backspace deletes a word (legacy byte encodings, as Terminal.app sends them).
#[test]
fn ctrl_arrows_and_ctrl_delete() {
    let d = TempDir::new();
    std::fs::write(d.file("k.txt"), "alpha beta gamma\nsecond line\nthird").unwrap();
    let mut h = Harness::start(&["edit", "k.txt"], d.path(), false, &[]);
    h.wait_for("alpha beta", 5);
    h.send(b"\x1b[1;5C"); // Ctrl+Right: end of line 1
    h.send(b"!");
    h.wait_for("gamma!", 2);
    h.send(b"\x1b[1;5D"); // Ctrl+Left: start of line
    h.send(b"[");
    h.wait_for("[alpha", 2);
    h.send(b"\x1b[1;5B"); // Ctrl+Down: end of document
    h.send(b"#");
    h.wait_for("third#", 2);
    h.send(b"\x1b[1;5A"); // Ctrl+Up: start of document
    h.send(b"^");
    h.wait_for("^[alpha", 2);
    h.send(b"\x1b[1;5C\x1b[D\x1b[D"); // end of line 1, back two chars (before "a!")
    h.send(b"\x08"); // Ctrl+Backspace (Terminal.app sends ^H): delete to line start
    h.wait_for("Ln 1, Col 1", 2);
    assert_eq!(h.row(1).split_whitespace().last(), Some("a!"), "{}", h.screen());
    h.send(b"\x1b[B\x1b[1;5D"); // line 2, start
    h.send(b"\x1b[3;5~"); // Ctrl+Delete: delete to line end
    h.send(b"\x1b\x7f"); // Alt+Backspace at column 0 joins nothing harmful
    h.send(b"\x13");
    let t0 = Instant::now();
    loop {
        let got = std::fs::read_to_string(d.file("k.txt")).unwrap();
        if got == "a!\nthird#" || got == "a!\n\nthird#" {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(3), "file: {got:?}\n{}", h.screen());
        h.pump(Duration::from_millis(30));
    }
    h.send(b"\x11");
    h.wait_exit();
}

/// Foreground color of the first cell of `needle` on screen.
fn fg_of(h: &Harness, needle: &str) -> Option<cmx_tui::Color> {
    for y in 0..30 {
        let row = &h.vt.grid()[y];
        let text: Vec<&str> = row.cells.iter().map(|c| c.sym.as_str()).collect();
        let line: String = text.concat();
        if let Some(byte) = line.find(needle) {
            let col = line[..byte].chars().count();
            return row.cells.get(col).map(|c| c.style.fg);
        }
    }
    None
}

/// stex keywords are colored by the stex lexer: a phrase only once complete, every alias alike.
#[test]
fn stex_highlighting_follows_the_compiler() {
    let d = TempDir::new();
    std::fs::write(d.file("p.stex"), "").unwrap();
    let mut h = Harness::start(&["edit", "p.stex"], d.path(), false, &[]);
    h.wait_for("p.stex", 5);
    h.send(b"let x in Z such");
    h.wait_for("such", 2);
    let plain = fg_of(&h, "x in").expect("x");
    assert_eq!(fg_of(&h, "such"), Some(plain), "\"such\" alone is not a keyword");
    h.send(b" that x > 0\rlet y in Z s.t. y > 0\rthere is y with y > 1");
    h.wait_for("y > 1", 2);
    let st = fg_of(&h, "such that").expect("such that");
    assert_ne!(st, plain, "\"such that\" is a keyword");
    assert_eq!(fg_of(&h, "s.t."), Some(st), "aliases share a color");
    assert_eq!(fg_of(&h, "there is"), Some(st), "quantifiers share the binder color");
    assert_ne!(fg_of(&h, "let"), Some(plain), "let is a keyword");
    h.send(b"\x11");
    h.wait_for("unsaved", 2);
    h.send(b"y");
    h.wait_exit();
}

#[test]
fn claude_pane_reports_missing_cli() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[("CMX_CLAUDE", "/nonexistent/claude")]);
    h.wait_for("⌨ legacy", 5);
    h.send(b"\x12"); // Ctrl+R == Ctrl+Shift+R (legacy)
    h.wait_for("CLAUDE", 2);
    h.wait_for("was not found", 2);
    h.send(b"\x11");
    h.wait_exit();
}

#[test]
fn resize_relayouts() {
    let d = TempDir::new();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("EDIT", 5);
    h.pty.resize(80, 20).unwrap();
    h.vt.resize(80, 20);
    h.pump(Duration::from_millis(300));
    assert!(h.row(19).contains("EDIT"), "status bar should move to the new last row:\n{}", h.screen());
    h.send(b"\x11");
    h.wait_exit();
}

#[test]
fn quick_open_and_command_palette() {
    let d = TempDir::new();
    std::fs::create_dir_all(d.file("deep/nested")).unwrap();
    std::fs::write(d.file("deep/nested/target_file.md"), "# Title here").unwrap();
    let mut h = Harness::start(&["edit"], d.path(), false, &[]);
    h.wait_for("⌨ legacy", 5);
    h.send(b"\x10"); // Ctrl+P (legacy → promoted to the command palette)
    h.wait_for("Command Palette", 2);
    h.send(b"\x7f"); // remove '>' → file mode
    h.wait_for("Go to File", 2);
    h.send(b"tgtfile");
    h.wait_for("deep/nested/target_file.md", 2);
    h.send(b"\r");
    h.wait_for("# Title here", 2);
    h.send(b"\x11");
    h.wait_exit();
}

#[test]
fn stex_new_creates_file() {
    let d = TempDir::new();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cmx"))
        .args(["stex", "new", "proof"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(d.file("proof.stex").exists());
    let again = std::process::Command::new(env!("CARGO_BIN_EXE_cmx"))
        .args(["stex", "new", "proof"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(!again.status.success(), "must refuse to overwrite");
}

#[test]
fn stex_new_untitled_saves_with_extension() {
    let d = TempDir::new();
    let mut h = Harness::start(&["stex", "new"], d.path(), false, &[]);
    h.wait_for("Untitled-1", 5);
    h.send(b"let x in integers");
    h.send(b"\x13");
    h.wait_for("Save as (.stex)", 2);
    h.send(b"myproof\r");
    h.wait_for("Saved myproof.stex", 2);
    h.send(b"\x11");
    h.wait_exit();
    assert_eq!(std::fs::read_to_string(d.file("myproof.stex")).unwrap(), "let x in integers");
}

/// Prints a text "screenshot" of the full IDE plus input latency numbers.
/// Run with: cargo test -p cmx --release --test e2e ide_screenshot -- --ignored --nocapture
#[test]
#[ignore]
fn ide_screenshot() {
    let d = TempDir::new();
    std::fs::create_dir_all(d.file("src")).unwrap();
    std::fs::write(
        d.file("src/main.rs"),
        "// demo\nfn main() {\n    let x = vec![1, 2, 3];\n    println!(\"{:?}\", x);\n}\n",
    )
    .unwrap();
    std::fs::write(d.file("README.md"), "# cmx\n").unwrap();
    let t0 = Instant::now();
    let mut h = Harness::start(&["workspace"], d.path(), false, &[("CMX_CLAUDE", "/bin/cat")]);
    h.wait_for("Open a file to begin", 5);
    let first_frame = t0.elapsed();
    h.send(b"\r"); // expand src/
    h.send(b"\x1b[B"); // → main.rs (preview)
    h.send(b"\r"); // pin + focus editor
    h.wait_for("println!", 3);
    h.send(b"\x12"); // claude pane (cat stands in for claude)
    h.wait_for("CLAUDE", 3);
    h.send(b"hello from the right pane\r");
    h.send(b"\x1b[17~\x1b[17~"); // F6 x2 → editor
    h.send(b"\x1b[17~"); // → terminal
    h.send(b"echo split-1\r");
    h.pump(Duration::from_millis(300));
    // latency: keypress → char visible
    h.send(b"\x1b[17~\x1b[17~\x1b[17~"); // terminal → claude → explorer → editor
    h.wait_for("Ln 1, Col 1", 2);
    let mut samples = Vec::new();
    let mut typed = String::new();
    for c in b"abcdefghij" {
        let t = Instant::now();
        h.write(&[*c]);
        typed.push(*c as char);
        loop {
            h.pump(Duration::from_millis(1));
            if h.row(1).contains(&typed) || t.elapsed() > Duration::from_millis(500) {
                break;
            }
        }
        samples.push(t.elapsed());
    }
    h.pump(Duration::from_millis(200));
    println!("{}", h.screen());
    samples.sort();
    println!(
        "first frame: {first_frame:?}; keypress→screen median {:?}, max {:?}",
        samples[samples.len() / 2],
        samples.last().unwrap()
    );
    h.send(b"\x11");
    h.wait_for("Quit anyway?", 2);
    h.send(b"y");
    h.wait_exit();
}

use super::*;
use cmx_tui::{KeyCode, KeyEvent, Mods, MouseButton, MouseEvent, MouseKind};

fn text(t: &Term, y: usize) -> String {
    let mut s = String::new();
    for c in &t.grid()[y].cells {
        if !c.sym.is_cont() {
            s.push_str(c.sym.as_str());
        }
    }
    s.trim_end().to_string()
}

#[test]
fn print_and_wrap() {
    let mut t = Term::new(5, 3);
    t.feed(b"hello world");
    assert_eq!(text(&t, 0), "hello");
    assert_eq!(text(&t, 1), " worl");
    assert_eq!(text(&t, 2), "d");
    assert!(t.grid()[0].wrapped);
    assert_eq!(t.cursor(), (1, 2));
}

#[test]
fn crlf_and_scrollback() {
    let mut t = Term::new(10, 2);
    t.feed(b"a\r\nb\r\nc\r\nd");
    assert_eq!(text(&t, 0), "c");
    assert_eq!(text(&t, 1), "d");
    assert_eq!(t.scrollback.len(), 2);
    assert_eq!(t.text_range((0, 0), (3, 1)), "a\nb\nc\nd");
}

#[test]
fn cursor_movement_and_erase() {
    let mut t = Term::new(10, 3);
    t.feed(b"abcdefghij\x1b[2;3Hxy\x1b[1;5H\x1b[K");
    assert_eq!(text(&t, 0), "abcd");
    assert_eq!(text(&t, 1), "  xy");
    t.feed(b"\x1b[2J");
    assert_eq!(text(&t, 0), "");
    assert_eq!(text(&t, 1), "");
}

#[test]
fn sgr_colors() {
    let mut t = Term::new(10, 1);
    t.feed(b"\x1b[1;31mA\x1b[38;2;1;2;3mB\x1b[38:5:200mC\x1b[0mD\x1b[48;5;17mE");
    let c = &t.grid()[0].cells;
    assert_eq!(c[0].style.fg, Color::Idx(1));
    assert!(c[0].style.has(attr::BOLD));
    assert_eq!(c[1].style.fg, Color::Rgb(1, 2, 3));
    assert_eq!(c[2].style.fg, Color::Idx(200));
    assert_eq!(c[3].style, Style::DEFAULT);
    assert_eq!(c[4].style.bg, Color::Idx(17));
}

#[test]
fn scroll_region_and_insert_delete_lines() {
    let mut t = Term::new(5, 4);
    t.feed(b"1\r\n2\r\n3\r\n4");
    t.feed(b"\x1b[2;3r"); // region rows 2..3
    t.feed(b"\x1b[3;1H\n"); // at bottom of region: scroll region up
    assert_eq!(text(&t, 0), "1");
    assert_eq!(text(&t, 1), "3");
    assert_eq!(text(&t, 2), "");
    assert_eq!(text(&t, 3), "4");
    t.feed(b"\x1b[r\x1b[1;1H\x1b[L");
    assert_eq!(text(&t, 0), "");
    assert_eq!(text(&t, 1), "1");
    t.feed(b"\x1b[M");
    assert_eq!(text(&t, 0), "1");
}

#[test]
fn alt_screen() {
    let mut t = Term::new(5, 2);
    t.feed(b"main");
    t.feed(b"\x1b[?1049h");
    assert!(t.is_alt());
    assert_eq!(text(&t, 0), "");
    t.feed(b"alt");
    t.feed(b"\x1b[?1049l");
    assert!(!t.is_alt());
    assert_eq!(text(&t, 0), "main");
    assert_eq!(t.cursor(), (4, 0));
}

#[test]
fn wide_and_combining() {
    let mut t = Term::new(6, 2);
    t.feed("a中e\u{301}".as_bytes());
    assert_eq!(text(&t, 0), "a中e\u{301}");
    assert_eq!(t.cursor(), (4, 0));
    // wide char at the last column wraps
    let mut t = Term::new(3, 2);
    t.feed("ab中".as_bytes());
    assert_eq!(text(&t, 0), "ab");
    assert_eq!(text(&t, 1), "中");
}

#[test]
fn replies() {
    let mut t = Term::new(10, 5);
    t.feed(b"\x1b[3;4H\x1b[6n\x1b[c");
    assert_eq!(t.responses, b"\x1b[3;4R\x1b[?62;22c");
    t.responses.clear();
    t.feed(b"\x1b]11;?\x07");
    assert_eq!(String::from_utf8_lossy(&t.responses), "\x1b]11;rgb:1e1e/1e1e/1e1e\x07");
    t.responses.clear();
    t.feed(b"\x1b[?2026$p");
    assert_eq!(t.responses, b"\x1b[?2026;2$y");
}

#[test]
fn osc_title_and_clipboard() {
    let mut t = Term::new(10, 2);
    t.feed(b"\x1b]0;my title\x07\x1b]52;c;aGVsbG8=\x1b\\x");
    assert_eq!(t.title, "my title");
    assert_eq!(t.clipboard.as_deref(), Some("hello"));
    assert_eq!(text(&t, 0), "x");
}

#[test]
fn dec_line_drawing() {
    let mut t = Term::new(10, 1);
    t.feed(b"\x1b(0lqk\x1b(Bx");
    assert_eq!(text(&t, 0), "┌─┐x");
}

#[test]
fn resize_keeps_cursor_visible() {
    let mut t = Term::new(10, 5);
    t.feed(b"1\r\n2\r\n3\r\n4\r\n5");
    t.resize(10, 3);
    assert_eq!(text(&t, 2), "5");
    assert_eq!(t.cursor().1, 2);
    assert_eq!(t.scrollback.len(), 2);
    t.resize(10, 5);
    assert_eq!(text(&t, 0), "1");
    assert_eq!(t.cursor().1, 4);
}

#[test]
fn key_encoding() {
    let mut t = Term::new(10, 2);
    assert_eq!(t.encode_key(&KeyEvent::plain(KeyCode::Up)), b"\x1b[A");
    t.feed(b"\x1b[?1h");
    assert_eq!(t.encode_key(&KeyEvent::plain(KeyCode::Up)), b"\x1bOA");
    assert_eq!(t.encode_key(&KeyEvent::new(KeyCode::Left, Mods::CTRL)), b"\x1b[1;5D");
    assert_eq!(t.encode_key(&KeyEvent::ctrl('c')), b"\x03");
    assert_eq!(t.encode_key(&KeyEvent::new(KeyCode::Char('b'), Mods::ALT)), b"\x1bb");
    assert_eq!(t.encode_key(&KeyEvent::new(KeyCode::Enter, Mods::SHIFT)), b"\x1b\r");
    assert_eq!(t.encode_key(&KeyEvent::plain(KeyCode::F(5))), b"\x1b[15~");
}

#[test]
fn mouse_and_paste_encoding() {
    let mut t = Term::new(10, 2);
    let ev = MouseEvent { kind: MouseKind::Down(MouseButton::Left), x: 0, y: 0, mods: Mods::NONE };
    assert_eq!(t.encode_mouse(&ev, 2, 3), None);
    t.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(t.encode_mouse(&ev, 2, 3).unwrap(), b"\x1b[<0;3;4M");
    assert_eq!(t.encode_paste("a\nb"), b"a\rb");
    t.feed(b"\x1b[?2004h");
    assert_eq!(t.encode_paste("a\nb"), b"\x1b[200~a\rb\x1b[201~");
}

#[test]
fn split_sequences_across_feeds() {
    let mut t = Term::new(10, 2);
    t.feed(b"\x1b[3");
    t.feed(b"1mR");
    t.feed("\u{4e2d}".as_bytes()[..1].as_ref());
    t.feed("\u{4e2d}".as_bytes()[1..].as_ref());
    assert_eq!(t.grid()[0].cells[0].style.fg, Color::Idx(1));
    assert_eq!(text(&t, 0), "R中");
}

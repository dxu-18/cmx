//! Draws the whole UI into the off-screen buffer (the diff renderer then updates the terminal).

use crate::app::{App, Drag, Focus, Hits, Kbd, MsgKind, PanelBtn};
use crate::highlight::{Tok, highlight_line};
use crate::keymap::{Cmd, PALETTE};
use crate::layout::{self, LayoutInput};
use crate::palette::PaletteMode;
use crate::search::Query;
use crate::termpane::PaneKind;
use crate::text::{Pos, grapheme_width};
use crate::theme::Theme;
use crate::vim::Mode;
use cmx_tui::unicode::{next_boundary, str_width};
use cmx_tui::{Buffer, Color, CursorShape, CursorState, Rect, Style, Sym, attr};

pub fn draw(app: &mut App) -> CursorState {
    let th: &'static Theme = app.theme;
    app.layout = layout::compute(&LayoutInput {
        width: app.width,
        height: app.height,
        show_tree: app.show_tree,
        tree_w: app.tree_w,
        show_claude: app.show_claude,
        claude_w: app.claude_w,
        show_panel: app.show_panel && !app.terms.is_empty(),
        panel_h: app.panel_h,
        term_ratios: &app.term_ratios,
    });
    // keep PTY sizes in sync with the layout
    let terms = app.layout.terms.clone();
    for (i, r) in terms.iter().enumerate() {
        if let Some(t) = app.terms.get_mut(i) {
            t.resize(r.w, r.h);
        }
    }
    if let (Some(c), Some(r)) = (app.claude.as_mut(), app.layout.claude_body) {
        c.resize(r.w, r.h);
    }
    app.hits = Hits::default();
    let mut buf = std::mem::replace(&mut app.buf, Buffer::new(0, 0));
    if buf.width != app.width || buf.height != app.height {
        buf.resize(app.width, app.height);
    }
    buf.clear(th.editor());
    let mut cursor = CursorState::default();

    draw_tree(app, &mut buf);
    let split_style = Style::new(th.split_fg, th.editor_bg);
    let hover_style = Style::new(th.split_focus_fg, th.editor_bg).bold();
    let l = app.layout.clone();
    let hover = app.split_highlight();
    for (r, d) in [(l.tree_split, Drag::TreeSplit), (l.claude_split, Drag::ClaudeSplit)] {
        if let Some(r) = r {
            buf.fill(
                r,
                if hover == Some(d) { '┃' } else { '│' },
                if hover == Some(d) { hover_style } else { split_style },
            );
        }
    }
    if let Some(c) = draw_editor(app, &mut buf) {
        cursor = c;
    }
    if let Some(c) = draw_panel(app, &mut buf) {
        cursor = c;
    }
    if let Some(c) = draw_claude(app, &mut buf) {
        cursor = c;
    }
    if let Some(c) = draw_status(app, &mut buf) {
        cursor = c;
    }
    // resize cursor drawn in the cell under the mouse (OSC 22 pointers aren't supported by every
    // terminal, e.g. Terminal.app): ↔ on vertical splitters, ↕ on the panel's height handle
    if let Some(d) = app.split_highlight() {
        let (x, y) = app.hover_at;
        if x < buf.width && y < buf.height {
            let glyph = if d == Drag::PanelSplit { '↕' } else { '↔' };
            buf.put_char(x, y, glyph, Style::new(th.status_fg, th.split_focus_fg).bold());
        }
    }
    if app.help {
        draw_help(app, &mut buf);
        cursor.visible = false;
    }
    if app.palette.is_some() {
        cursor = draw_palette(app, &mut buf);
    }
    app.buf = buf;
    cursor
}

fn tok_style(th: &Theme, t: Tok, base: Style) -> Style {
    let fg = match t {
        Tok::Text => return base,
        Tok::Keyword => th.syn_keyword,
        Tok::Control => th.syn_control,
        Tok::Type => th.syn_type,
        Tok::Function => th.syn_function,
        Tok::Str | Tok::Code => th.syn_string,
        Tok::Number => th.syn_number,
        Tok::Comment => return base.fg(th.syn_comment).italic(),
        Tok::Constant => th.syn_constant,
        Tok::Macro => th.syn_macro,
        Tok::Operator => th.syn_operator,
        Tok::Heading => return base.fg(th.syn_heading).bold(),
        Tok::Variable => th.syn_variable,
        Tok::Tag => th.syn_tag,
        Tok::Math => th.syn_math,
        Tok::Emph => return base.italic(),
        Tok::Strong => return base.bold(),
        Tok::Link => return base.fg(th.syn_constant).underline(),
    };
    base.fg(fg)
}

fn pad_right(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, limit: u16) -> u16 {
    buf.put_str(x, y, s, style, limit)
}

// ---------------------------------------------------------------------- explorer

fn draw_tree(app: &mut App, buf: &mut Buffer) {
    let Some(r) = app.layout.tree else { return };
    let th = app.theme;
    let focused = app.focus == Focus::Tree;
    buf.fill(r, ' ', th.sidebar());
    let head = Style::new(if focused { th.header_focus_fg } else { th.header_fg }, th.header_bg).bold();
    buf.fill(r.row(0), ' ', head);
    let title = format!(" EXPLORER: {}", app.tree.root_name().to_uppercase());
    buf.put_str(r.x, r.y, &title, head, r.right().saturating_sub(5));
    let bx = r.right().saturating_sub(4);
    buf.put_str(bx, r.y, "⟳", head, r.right());
    buf.put_str(bx + 2, r.y, "×", head, r.right());
    app.hits.buttons.push((Rect::new(bx, r.y, 1, 1), PanelBtn::RefreshTree));
    app.hits.buttons.push((Rect::new(bx + 2, r.y, 1, 1), PanelBtn::HideTree));
    let h = r.h.saturating_sub(1) as usize;
    if std::mem::take(&mut app.tree.pending_reveal) {
        app.tree.ensure_visible(h);
    }
    let max_scroll = app.tree.nodes.len().saturating_sub(h);
    app.tree.scroll = app.tree.scroll.min(max_scroll);
    let open = app.tabs.get(app.active).and_then(|t| t.doc.path.clone());
    for row in 0..h {
        let i = app.tree.scroll + row;
        let Some(n) = app.tree.nodes.get(i) else { break };
        let y = r.y + 1 + row as u16;
        let selected = i == app.tree.selected;
        let bg =
            if selected { if focused { th.sidebar_sel_focus_bg } else { th.sidebar_sel_bg } } else { th.sidebar_bg };
        let is_open = open.as_deref() == Some(n.path.as_path());
        let fg = if n.is_dir {
            th.sidebar_dir_fg
        } else if is_open {
            th.tab_active_fg
        } else {
            th.sidebar_fg
        };
        let st = Style::new(fg, bg);
        buf.fill(Rect::new(r.x, y, r.w, 1), ' ', st);
        let indent = (n.depth * 2 + 1) as u16;
        let icon = if n.is_dir { if n.expanded { "▾ " } else { "▸ " } } else { "  " };
        let x = buf.put_str(r.x + indent, y, icon, st.fg(th.dim_fg), r.right());
        let name_style = if n.is_dir { st.bold() } else { st };
        buf.put_str(x, y, &n.name, name_style, r.right());
    }
    if app.tree.nodes.is_empty() {
        buf.put_str(r.x + 2, r.y + 2, "(empty folder)", th.sidebar().fg(th.dim_fg), r.right());
    }
}

// ---------------------------------------------------------------------- editor

const WELCOME: &[(&str, &str)] = &[
    ("Ctrl+P", "Quick open file"),
    ("Ctrl+N", "New untitled file"),
    ("Ctrl+Shift+L", "Toggle explorer"),
    ("Ctrl+Shift+Y", "Toggle terminal"),
    ("Ctrl+Shift+R", "Toggle Claude"),
    ("Ctrl+K", "Toggle vim keys"),
    ("F1", "All keyboard shortcuts"),
];

fn draw_editor(app: &mut App, buf: &mut Buffer) -> Option<CursorState> {
    let th = app.theme;
    let l = app.layout.clone();
    if l.editor.is_empty() {
        return None;
    }
    // tab bar
    buf.fill(l.tabbar, ' ', Style::new(th.tab_fg, th.tabbar_bg));
    if app.tabs.is_empty() {
        let r = l.text;
        let st = th.editor().fg(th.dim_fg);
        let total = WELCOME.len() as u16 + 3;
        let y0 = r.y + r.h.saturating_sub(total) / 2;
        let msg = "Open a file to begin";
        let cx = |w: usize| r.x + r.w.saturating_sub(w as u16) / 2;
        buf.put_str(cx(msg.len()), y0, msg, th.editor().fg(th.editor_fg).bold(), r.right());
        for (i, (k, d)) in WELCOME.iter().enumerate() {
            let line = format!("{k:<14}{d}");
            let w = 14 + 24;
            let x = cx(w);
            let y = y0 + 2 + i as u16;
            if y >= r.bottom() {
                break;
            }
            buf.put_str(x, y, &format!("{k:<14}"), st.fg(th.syn_function), r.right());
            buf.put_str(x + 14, y, d, st, r.right());
            let _ = line;
        }
        return None;
    }
    // tabs: compute widths and a horizontal offset that keeps the active tab visible
    let labels: Vec<(String, bool, bool)> = app
        .tabs
        .iter()
        .map(|t| {
            let mut name = t.doc.display_name();
            if t.doc.deleted {
                name.push_str(" (deleted)");
            } else if t.doc.conflict {
                name.push_str(" ⚠");
            }
            (name, t.doc.is_dirty(), t.preview)
        })
        .collect();
    let widths: Vec<u16> = labels.iter().map(|(n, _, _)| str_width(n) as u16 + 4).collect();
    let mut start = 0usize;
    loop {
        let w: u16 = widths[start..=app.active.max(start)].iter().sum();
        if w <= l.tabbar.w || start >= app.active {
            break;
        }
        start += 1;
    }
    let mut x = l.tabbar.x;
    for (i, (name, dirty, preview)) in labels.iter().enumerate().skip(start) {
        if x >= l.tabbar.right() {
            break;
        }
        let active = i == app.active;
        let mut st =
            if active { Style::new(th.tab_active_fg, th.tab_active_bg) } else { Style::new(th.tab_fg, th.tab_bg) };
        if *preview {
            st = st.italic();
        }
        let w = widths[i];
        let r = Rect::new(x, l.tabbar.y, w.min(l.tabbar.right() - x), 1);
        buf.fill(r, ' ', st);
        buf.put_str(x + 1, r.y, name, st, r.right());
        let cx = x + w - 2;
        let (mark, mst) = if *dirty {
            ("●", st.fg(th.tab_dirty_fg))
        } else {
            ("×", st.fg(if active { th.tab_active_fg } else { th.dim_fg }))
        };
        if cx < l.tabbar.right() {
            buf.put_str(cx, r.y, mark, mst, l.tabbar.right());
            app.hits.tab_close.push((Rect::new(cx, r.y, 1, 1), i));
        }
        app.hits.tabs.push((r, i));
        x += w + 1;
    }
    draw_text(app, buf, l.text)
}

/// One screen row of the text area: a whole line, or one wrap segment of it.
struct Row {
    line: usize,
    /// Byte range of the line shown on this row.
    b0: usize,
    b1: usize,
    /// Display column of `b0` (the row's horizontal origin when wrapping).
    dc0: usize,
    first: bool,
    last: bool,
}

fn draw_text(app: &mut App, buf: &mut Buffer, r: Rect) -> Option<CursorState> {
    let th = app.theme;
    let focused = app.focus == Focus::Editor && app.prompt.is_none() && app.palette.is_none();
    let vim_block = app.vim_on && app.vim.mode != Mode::Insert;
    let query =
        if app.search_hl { app.search.as_deref().and_then(|q| Query::new(q, app.search_flags).ok()) } else { None };
    let tab = app.tabs.get_mut(app.active)?;
    let nlines = tab.doc.text.len_lines();
    let digits = nlines.to_string().len().max(3) as u16;
    let gutter_w = digits + 2;
    let show_sb = nlines > r.h as usize;
    let text_x = r.x + gutter_w;
    let text_w = r.w.saturating_sub(gutter_w + show_sb as u16);
    tab.view.height = r.h as usize;
    tab.view.width = text_w as usize;
    tab.view.scroll_y = tab.view.scroll_y.min(nlines.saturating_sub(1));
    let wrap = tab.view.wrap;
    if wrap {
        tab.view.scroll_x = 0;
    }
    app.hits.gutter_w = gutter_w;
    let tw = tab.doc.tab_width;
    let sel = tab.view.selection_in(&tab.doc.text);
    let cur = tab.view.cursor;
    let top = tab.view.scroll_y;
    // the visible rows
    let mut rows: Vec<Row> = Vec::with_capacity(r.h as usize);
    if wrap {
        let mut l = top;
        let mut sub = tab.view.scroll_sub;
        while rows.len() < r.h as usize && l < nlines {
            let s = tab.doc.text.line(l);
            let segs = crate::view::wrap_starts(s, text_w as usize, tw);
            let sub0 = sub.min(segs.len() - 1);
            if l == top {
                tab.view.scroll_sub = sub0;
            }
            for k in sub0..segs.len() {
                if rows.len() >= r.h as usize {
                    break;
                }
                let b1 = segs.get(k + 1).map_or(s.len(), |x| x.0);
                rows.push(Row { line: l, b0: segs[k].0, b1, dc0: segs[k].1, first: k == 0, last: k + 1 == segs.len() });
            }
            sub = 0;
            l += 1;
        }
    } else {
        for line in top..(top + r.h as usize).min(nlines) {
            let len = tab.doc.text.line_len(line);
            rows.push(Row { line, b0: 0, b1: len, dc0: 0, first: true, last: true });
        }
    }
    let bottom = rows.last().map_or(top, |x| x.line + 1);
    // bracket pair near the cursor
    let mut brackets: Vec<Pos> = Vec::new();
    if focused {
        let t = &tab.doc.text;
        let at = t.match_bracket(cur).map(|m| (cur, m)).or_else(|| {
            if cur.col > 0 {
                let p = t.prev_pos(cur);
                t.match_bracket(p).map(|m| (p, m))
            } else {
                None
            }
        });
        if let Some((a, b)) = at {
            brackets.push(a);
            brackets.push(b);
        }
    }
    let mut matches = Vec::new();
    if let Some(q) = &query {
        q.matches(&tab.doc.text, top, bottom, &mut matches);
    }
    // secondary cursors and their selections
    let extra_sel: Vec<(Pos, Pos)> = tab
        .view
        .extra
        .iter()
        .filter_map(|&(c, a)| a.filter(|a| *a != c).map(|a| if a < c { (a, c) } else { (c, a) }))
        .collect();
    let extra_cur: Vec<Pos> = tab.view.extra.iter().map(|s| s.0).collect();
    let cursor_style = Style::new(th.editor_bg, th.editor_fg);
    let lang = tab.doc.lang;
    let mut spans = Vec::new();
    let mut spans_line = usize::MAX;
    let mut cursor_xy: Option<(u16, u16)> = None;
    for (ri, row) in rows.iter().enumerate() {
        let line = row.line;
        let y = r.y + ri as u16;
        let s = tab.doc.text.line(line);
        let is_cur = line == cur.line;
        let cur_here = is_cur && cur.col >= row.b0 && (cur.col < row.b1 || row.last);
        let line_bg = if cur_here && sel.is_none() { th.cursor_line_bg } else { th.editor_bg };
        // gutter: the line number on the first row of a line
        let gst = Style::new(if is_cur { th.gutter_active_fg } else { th.gutter_fg }, th.editor_bg);
        if row.first {
            let num = format!("{:>width$} ", line + 1, width = digits as usize + 1);
            buf.put_str(r.x, y, &num, gst, text_x);
        }
        buf.fill(Rect::new(text_x, y, text_w, 1), ' ', Style::new(th.editor_fg, line_bg));
        // highlight (once per line)
        if spans_line != line {
            spans.clear();
            let st = tab.doc.hl.state_at(lang, tab.doc.text.lines(), line);
            if s.len() < 20_000 {
                highlight_line(lang, s, st, Some(&mut spans));
            }
            spans_line = line;
        }
        let (ss, se, nl_sel) = match sel {
            Some((a, b)) if a.line <= line && b.line >= line => {
                let ss = if a.line == line { a.col } else { 0 };
                let se = if b.line == line { b.col } else { s.len() };
                (ss, se, b.line > line)
            }
            _ => (usize::MAX, 0, false),
        };
        let line_matches: Vec<(usize, usize)> =
            matches.iter().filter(|(a, _)| a.line == line).map(|(a, b)| (a.col, b.col)).collect();
        let line_extra: Vec<(usize, usize)> = extra_sel
            .iter()
            .filter(|(a, b)| a.line <= line && b.line >= line)
            .map(|(a, b)| (if a.line == line { a.col } else { 0 }, if b.line == line { b.col } else { usize::MAX }))
            .collect();
        let x_off = if wrap { row.dc0 } else { tab.view.scroll_x };
        let mut dcol = row.dc0;
        let mut i = row.b0;
        let mut si = 0usize;
        let limit = x_off + text_w as usize;
        let screen_x = |dc: usize| text_x + (dc - x_off) as u16;
        while i < row.b1 {
            let j = next_boundary(s, i);
            let g = &s[i..j];
            let w = grapheme_width(g, dcol, tw);
            if cur_here && i == cur.col && dcol >= x_off && dcol < limit {
                cursor_xy = Some((screen_x(dcol), y));
            }
            if dcol + w > x_off && dcol < limit {
                while si < spans.len() && (spans[si].1 as usize) <= i {
                    si += 1;
                }
                let mut style = Style::new(th.editor_fg, line_bg);
                if let Some(&(a, _, t)) = spans.get(si)
                    && (a as usize) <= i
                {
                    style = tok_style(th, t, style);
                }
                if let Some(&(a, b)) = line_matches.iter().find(|(a, b)| i >= *a && i < *b) {
                    let current = sel.is_some_and(|(sa, sb)| sa == Pos::new(line, a) && sb == Pos::new(line, b));
                    style.bg = if current { th.search_cur_bg } else { th.search_bg };
                }
                if brackets.contains(&Pos::new(line, i)) {
                    style.bg = th.bracket_bg;
                    style.attrs |= attr::BOLD;
                }
                if (i >= ss && i < se) || line_extra.iter().any(|&(a, b)| i >= a && i < b) {
                    style.bg = th.selection_bg;
                }
                if focused && extra_cur.contains(&Pos::new(line, i)) {
                    style = cursor_style;
                }
                let x0 = text_x as usize + dcol.saturating_sub(x_off);
                let b0 = g.as_bytes()[0];
                if b0 == b'\t' || dcol < x_off {
                    let vis = (dcol + w).min(limit) - dcol.max(x_off);
                    for k in 0..vis {
                        buf.set(x0 as u16 + k as u16, y, Sym::BLANK, style, text_x + text_w);
                    }
                } else if b0 < 0x20 || b0 == 0x7f {
                    let c = if b0 == 0x7f { '?' } else { (b0 + b'@') as char };
                    let cs = style.fg(th.dim_fg);
                    let nx = buf.set(x0 as u16, y, Sym::from_char('^'), cs, text_x + text_w);
                    buf.set(nx, y, Sym::from_char(c), cs, text_x + text_w);
                } else {
                    buf.set(x0 as u16, y, Sym::from_cluster(g), style, text_x + text_w);
                }
            }
            dcol += w;
            i = j;
            if !wrap && dcol >= limit && !(cur_here && cur.col >= i) {
                break;
            }
        }
        // cursors at the end of the line (or beyond the drawn part)
        let end_dc = |col: usize| crate::text::display_width_upto(s, col.min(s.len()), tw);
        if cur_here && cursor_xy.is_none() {
            let dc = end_dc(cur.col);
            if dc >= x_off && dc < limit + usize::from(wrap) {
                let x = screen_x(dc).min(text_x + text_w.saturating_sub(1));
                cursor_xy = Some((x, y));
            }
        }
        if row.last {
            let eol = end_dc(s.len());
            let eol_x = (eol >= x_off && eol < limit).then(|| screen_x(eol));
            if let Some(x) = eol_x {
                if nl_sel || line_extra.iter().any(|&(_, b)| b == usize::MAX) {
                    buf.get_mut(x, y).style.bg = th.selection_bg;
                }
                if focused && extra_cur.contains(&Pos::new(line, s.len())) {
                    buf.get_mut(x, y).style = cursor_style;
                }
            }
        }
    }
    // scrollbar
    if show_sb {
        let sb = Rect::new(r.right() - 1, r.y, 1, r.h);
        let h = r.h as usize;
        let thumb_h = (h * h / nlines).max(1);
        let thumb_y = (top * h / nlines).min(h - thumb_h);
        buf.fill(sb, ' ', Style::new(th.editor_fg, th.editor_bg));
        buf.fill(
            Rect::new(sb.x, sb.y + thumb_y as u16, 1, thumb_h as u16),
            ' ',
            Style::new(th.editor_fg, th.scrollbar),
        );
        app.hits.scrollbar = Some(sb);
    }
    if !focused {
        return None;
    }
    let (x, y) = cursor_xy?;
    Some(CursorState { x, y, shape: if vim_block { CursorShape::Block } else { CursorShape::Bar }, visible: true })
}

// ---------------------------------------------------------------------- terminal panel / claude

fn cursor_shape(decscusr: u8) -> CursorShape {
    match decscusr {
        3 | 4 => CursorShape::Underline,
        5 | 6 => CursorShape::Bar,
        _ => CursorShape::Block,
    }
}

fn draw_panel(app: &mut App, buf: &mut Buffer) -> Option<CursorState> {
    let th = app.theme;
    let l = app.layout.clone();
    let (Some(_p), Some(hr)) = (l.panel, l.panel_header) else { return None };
    let term_focused = matches!(app.focus, Focus::Term(_));
    let head = Style::new(if term_focused { th.header_focus_fg } else { th.header_fg }, th.header_bg);
    buf.fill(hr, ' ', head);
    // a thin top border look
    let mut x = buf.put_str(hr.x, hr.y, " TERMINAL ", head.bold(), hr.right());
    for (i, t) in app.terms.iter().enumerate() {
        let label = format!(" {}: {} ", i + 1, t.title());
        let label: String = label.chars().take(24).collect();
        let active = i == app.active_term;
        let st = if active && term_focused {
            Style::new(th.tab_active_fg, th.sidebar_sel_focus_bg)
        } else if active {
            Style::new(th.tab_active_fg, th.sidebar_sel_bg)
        } else {
            head
        };
        let w = str_width(&label) as u16;
        if x + w + 8 >= hr.right() {
            break;
        }
        buf.put_str(x, hr.y, &label, st, hr.right());
        app.hits.term_tabs.push((Rect::new(x, hr.y, w, 1), i));
        x += w + 1;
    }
    let bx = hr.right().saturating_sub(8);
    for (k, (g, b)) in [("+", PanelBtn::New), ("×", PanelBtn::Kill), ("▾", PanelBtn::Hide)].into_iter().enumerate() {
        let bxk = bx + 1 + k as u16 * 2;
        buf.put_str(bxk, hr.y, g, head.bold(), hr.right());
        app.hits.buttons.push((Rect::new(bxk, hr.y, 1, 1), b));
    }
    let split_style = Style::new(th.split_fg, th.term_bg);
    let hover = app.split_highlight();
    for (i, r) in l.term_splits.iter().enumerate() {
        if hover == Some(Drag::TermSplit(i)) {
            buf.fill(*r, '┃', Style::new(th.split_focus_fg, th.term_bg).bold());
        } else {
            buf.fill(*r, '│', split_style);
        }
    }
    if hover == Some(Drag::PanelSplit) {
        // the header row is the panel's resize handle: mark its top edge
        for x in hr.x..hr.right() {
            let c = buf.get_mut(x, hr.y);
            c.style.attrs |= attr::UNDERLINE;
            c.style.fg = th.split_focus_fg;
        }
    }
    let mut cur = None;
    for (i, r) in l.terms.iter().enumerate() {
        let Some(t) = app.terms.get(i) else { continue };
        let focused = app.focus == Focus::Term(i) && app.prompt.is_none() && app.palette.is_none();
        if let Some((cx, cy, shape)) = t.render(buf, *r, th, focused) {
            cur = Some(CursorState { x: cx, y: cy, shape: cursor_shape(shape), visible: true });
        }
    }
    cur
}

fn draw_claude(app: &mut App, buf: &mut Buffer) -> Option<CursorState> {
    let th = app.theme;
    let l = app.layout.clone();
    let (Some(hr), Some(body)) = (l.claude_header, l.claude_body) else { return None };
    let focused = app.focus == Focus::Claude;
    let head = Style::new(if focused { th.header_focus_fg } else { th.header_fg }, th.header_bg);
    buf.fill(hr, ' ', head);
    let title = match &app.claude {
        Some(c) if !c.vt.title.is_empty() => format!(" CLAUDE · {}", c.vt.title),
        _ => " CLAUDE".to_string(),
    };
    buf.put_str(hr.x, hr.y, &title, head.bold(), hr.right().saturating_sub(6));
    let bx = hr.right().saturating_sub(5);
    buf.put_str(bx + 1, hr.y, "⟳", head.bold(), hr.right());
    buf.put_str(bx + 3, hr.y, "×", head.bold(), hr.right());
    app.hits.buttons.push((Rect::new(bx + 1, hr.y, 1, 1), PanelBtn::RestartClaude));
    app.hits.buttons.push((Rect::new(bx + 3, hr.y, 1, 1), PanelBtn::HideClaude));
    let Some(c) = &app.claude else {
        buf.fill(body, ' ', th.term());
        return None;
    };
    debug_assert_eq!(c.kind, PaneKind::Claude);
    let focused = focused && app.prompt.is_none() && app.palette.is_none();
    c.render(buf, body, th, focused).map(|(x, y, s)| CursorState { x, y, shape: cursor_shape(s), visible: true })
}

// ---------------------------------------------------------------------- status bar

fn draw_status(app: &mut App, buf: &mut Buffer) -> Option<CursorState> {
    let th = app.theme;
    let r = app.layout.status;
    if r.is_empty() {
        return None;
    }
    let st = th.status();
    buf.fill(r, ' ', st);
    if let Some(p) = &app.prompt {
        let pst = Style::new(th.editor_fg, th.header_bg);
        buf.fill(r, ' ', pst);
        let x = buf.put_str(r.x, r.y, &p.label, pst.bold(), r.right());
        // scroll the input if it is wider than the space left
        let avail = r.right().saturating_sub(x + 1) as usize;
        let cw = p.input.cursor_width();
        let skip = cw.saturating_sub(avail);
        let mut shown = String::new();
        let mut w = 0;
        for ch in p.input.text.chars() {
            let cwid = cmx_tui::unicode::char_width(ch);
            if w >= skip {
                shown.push(ch);
            }
            w += cwid;
        }
        buf.put_str(x, r.y, &shown, pst, r.right());
        if p.is_confirm() {
            return Some(CursorState { x: x.min(r.right() - 1), y: r.y, shape: CursorShape::Bar, visible: true });
        }
        return Some(CursorState {
            x: (x as usize + cw - skip).min(r.right() as usize - 1) as u16,
            y: r.y,
            shape: CursorShape::Bar,
            visible: true,
        });
    }
    // mode badge
    let (badge, bbg) = if app.vim_on {
        let bg = match app.vim.mode {
            Mode::Normal => th.status_vim_normal_bg,
            Mode::Insert => th.status_vim_insert_bg,
            _ => th.status_vim_visual_bg,
        };
        (format!(" VIM {} ", app.vim.mode_label()), bg)
    } else {
        (" EDIT ".to_string(), th.status_vim_normal_bg)
    };
    let bst = Style::new(th.status_fg, bbg).bold();
    let mut x = buf.put_str(r.x, r.y, &badge, bst, r.right());
    app.hits.status_mode = Rect::new(r.x, r.y, x - r.x, 1);
    // what has focus
    let what = match app.focus {
        Focus::Editor => app.tabs.get(app.active).map(|t| {
            let mut s = format!(" {}", t.doc.display_name());
            if t.doc.is_dirty() {
                s.push_str(" ●");
            }
            s.push(' ');
            s
        }),
        Focus::Tree => Some(" EXPLORER — ↑↓ browse · Enter open · a new · r rename · d delete ".to_string()),
        Focus::Term(i) => app.terms.get(i).map(|t| format!(" TERMINAL {} — F6/click to leave ", t.title())),
        Focus::Claude => Some(" CLAUDE — F6/click to leave ".to_string()),
    };
    if let Some(w) = what {
        x = buf.put_str(x, r.y, &w, st, r.right());
    }
    // right side
    let mut right = String::new();
    if app.vim_on {
        let p = app.vim.pending_str();
        if !p.is_empty() {
            right.push_str(&format!("{p}   "));
        }
    }
    if app.focus == Focus::Editor
        && let Some(t) = app.tabs.get(app.active)
    {
        let c = t.view.cursor;
        let dc = t.doc.text.display_col(c.line, c.col, t.doc.tab_width);
        right.push_str(&format!("Ln {}, Col {}", c.line + 1, dc + 1));
        if let Some((a, b)) = t.view.selection_in(&t.doc.text) {
            let n = t.doc.text.slice(a, b).chars().count();
            right.push_str(&format!(" ({n} selected)"));
        }
        right.push_str(&format!(
            "   {}   UTF-8   {}   {}",
            if t.doc.use_tabs { "Tabs".to_string() } else { format!("Spaces: {}", t.doc.tab_width) },
            if t.doc.line_ending == crate::text::LineEnding::CrLf { "CRLF" } else { "LF" },
            t.doc.lang.name()
        ));
    }
    let kbd = match app.kbd {
        Kbd::Kitty => "  ⌨ kitty ",
        Kbd::Legacy if app.seen_enhanced => "  ⌨ xterm ",
        Kbd::Legacy if app.mod_probe => "  ⌨ legacy+⇧ ",
        Kbd::Legacy => "  ⌨ legacy ",
        Kbd::Unknown => " ",
    };
    right.push_str(kbd);
    let rw = str_width(&right) as u16;
    let rx = r.right().saturating_sub(rw);
    // message between left and right parts
    if let Some((m, kind, _)) = &app.message {
        let fg = match kind {
            MsgKind::Info => th.status_fg,
            MsgKind::Warn => th.status_warn_fg,
            MsgKind::Error => th.status_err_fg,
        };
        let mst = Style::new(fg, th.status_bg);
        let x2 =
            buf.put_str(x + 1, r.y, m, if *kind == MsgKind::Info { mst } else { mst.bold() }, rx.saturating_sub(1));
        let _ = x2;
    }
    if rx > x {
        pad_right(buf, rx, r.y, &right, st, r.right());
    }
    None
}

// ---------------------------------------------------------------------- overlays

fn draw_box(buf: &mut Buffer, r: Rect, th: &Theme, title: &str) {
    let st = th.overlay();
    let bst = Style::new(th.overlay_border, th.overlay_bg);
    buf.fill(r, ' ', st);
    if r.w < 2 || r.h < 2 {
        return;
    }
    for x in r.x + 1..r.right() - 1 {
        buf.put_char(x, r.y, '─', bst);
        buf.put_char(x, r.bottom() - 1, '─', bst);
    }
    for y in r.y + 1..r.bottom() - 1 {
        buf.put_char(r.x, y, '│', bst);
        buf.put_char(r.right() - 1, y, '│', bst);
    }
    buf.put_char(r.x, r.y, '╭', bst);
    buf.put_char(r.right() - 1, r.y, '╮', bst);
    buf.put_char(r.x, r.bottom() - 1, '╰', bst);
    buf.put_char(r.right() - 1, r.bottom() - 1, '╯', bst);
    if !title.is_empty() {
        buf.put_str(r.x + 2, r.y, &format!(" {title} "), st.bold(), r.right() - 2);
    }
}

fn draw_palette(app: &mut App, buf: &mut Buffer) -> CursorState {
    let th = app.theme;
    let w = app.width.saturating_sub(4).min(90);
    let x = (app.width - w) / 2;
    let max_rows = app.height.saturating_sub(6).min(16) as usize;
    let Some(p) = &app.palette else { return CursorState::default() };
    let n = p.results.len().min(max_rows).max(1);
    let r = Rect::new(x, 1, w, n as u16 + 3);
    let title = match p.mode() {
        PaletteMode::Commands => "Command Palette",
        PaletteMode::Files => "Go to File  (type > for commands, : for line)",
    };
    draw_box(buf, r, th, title);
    app.hits.palette_box = Some(r);
    let inner = r.inset(2, 1, 2, 1);
    let ist = Style::new(th.editor_fg, th.editor_bg);
    buf.fill(inner.row(0), ' ', ist);
    let ix = buf.put_str(inner.x, inner.y, &p.input.text, ist, inner.right());
    let _ = ix;
    let cursor =
        CursorState { x: inner.x + p.input.cursor_width() as u16, y: inner.y, shape: CursorShape::Bar, visible: true };
    let mut rows = Vec::new();
    if p.results.is_empty() {
        let msg = if p.input.text.starts_with(':') { "Press Enter to go to the line" } else { "No matching results" };
        buf.put_str(inner.x, inner.y + 1, msg, th.overlay().fg(th.dim_fg), inner.right());
    }
    for (k, (_, idx, mpos)) in p.results.iter().skip(p.scroll).take(max_rows).enumerate() {
        let y = inner.y + 1 + k as u16;
        let selected = p.scroll + k == p.selected;
        let bg = if selected { th.overlay_sel_bg } else { th.overlay_bg };
        let st = Style::new(th.overlay_fg, bg);
        buf.fill(Rect::new(r.x + 1, y, r.w - 2, 1), ' ', st);
        let label = p.label(*idx);
        let mut cx = inner.x;
        for (bi, ch) in label.char_indices() {
            let s = if mpos.contains(&bi) { st.fg(th.overlay_match_fg).bold() } else { st };
            cx = buf.put_str(cx, y, ch.encode_utf8(&mut [0; 4]), s, inner.right());
        }
        if p.mode() == PaletteMode::Commands {
            let cmd: Cmd = PALETTE[*idx].1;
            if let Some(key) = app.keymap.label(cmd) {
                let kx = inner.right().saturating_sub(str_width(&key) as u16);
                if kx > cx + 1 {
                    buf.put_str(kx, y, &key, st.fg(th.dim_fg), inner.right());
                }
            }
        }
        rows.push((Rect::new(r.x + 1, y, r.w - 2, 1), p.scroll + k));
    }
    app.hits.palette_rows = rows;
    cursor
}

/// Help rows (Ctrl is the command modifier; Cmd is left to macOS, EDITOR-DESIGN D-16).
const HELP: &[(&str, &str)] = &[
    ("", "FILES & TABS"),
    ("Ctrl+S / Ctrl+Shift+S", "Save / Save as"),
    ("Ctrl+N / Ctrl+O / Ctrl+P", "New / Open path / Quick open (fuzzy)"),
    ("Ctrl+W / Ctrl+Q", "Close tab / Quit (Ctrl+Shift+Q inside a terminal pane)"),
    ("Ctrl+PgUp/PgDn, Alt+1..9", "Previous/next tab, go to tab N"),
    ("Ctrl+Shift+P", "Command palette (or Ctrl+P then >)"),
    ("", "PANELS"),
    ("Ctrl+Shift+L  (Ctrl+B)", "Explorer: show → focus → hide"),
    ("Ctrl+Shift+Y  (Ctrl+`)", "Terminal: show → focus → hide"),
    ("Ctrl+Shift+R", "Claude: show → focus → hide"),
    ("Ctrl+Shift+5, header +", "Split terminal"),
    ("F6", "Cycle focus (works inside terminals)"),
    ("Mouse", "Click to focus, drag borders to resize (they light up on hover)"),
    ("Files changed on disk", "Reload automatically (Ctrl+Z undoes); edited buffers are flagged ⚠"),
    ("", "EDITING"),
    ("Ctrl+Z / Ctrl+Shift+Z", "Undo / Redo (also Ctrl+Y, Alt+Shift+Z, F4)"),
    ("Ctrl+X / C / V / A", "Cut / Copy / Paste / Select all"),
    ("Ctrl+F / Ctrl+H", "Find (Enter next, Shift+Enter prev, Tab → replace) / Replace"),
    ("  Alt+R / Alt+C / Alt+W", "In Find: regex / match case / whole word ($1 in replacements)"),
    ("F3 / Shift+F3", "Find next / previous"),
    ("Ctrl+G", "Go to line[:col]"),
    ("Ctrl+D / Ctrl+L", "Add next occurrence (multiple cursors) / select line"),
    ("Ctrl+F2", "Select all occurrences (a cursor at each)"),
    ("Ctrl+Alt+↑/↓, Alt+click", "Add a cursor above / below / at the click"),
    ("Esc", "Back to one cursor"),
    ("Alt+Z", "Toggle word wrap (on by default for .md/.tex/.stex/.txt)"),
    ("Ctrl+/", "Toggle line comment"),
    ("Alt+↑/↓, Alt+Shift+↑/↓", "Move / copy line"),
    ("Ctrl+Shift+K", "Delete line"),
    ("Ctrl+Enter / +Shift", "Insert line below / above"),
    ("Tab / Shift+Tab", "Indent / outdent"),
    ("Ctrl+Shift+\\", "Jump to matching bracket"),
    ("Ctrl+←/→, Ctrl+↑/↓", "Line start / end, document start / end (Shift selects)"),
    ("Alt+←/→", "Word left / right (Shift selects)"),
    ("Ctrl+Backspace / Ctrl+Delete", "Delete to line start / end"),
    ("Alt+Backspace / Alt+Delete", "Delete word left / right"),
    ("Mouse", "Drag select, double-click word, triple-click line, Ctrl+click extends"),
    ("", "VIM MODE (Ctrl+K)"),
    ("Motions", "h j k l w b e W B E 0 ^ $ gg G f F t T ; , % { } H M L"),
    ("Operators", "d c y > < g~ gu gU + motion / text object (iw aw i\" a( i{ ...)"),
    ("Commands", "i a I A o O x X s S r ~ D C Y p P J u Ctrl+R . v V / ? n N * #"),
    ("Ex", ":w :q :wq :q! :e f :N :s/a/b/g :%s/a/b/g :noh :term :tree :claude"),
    ("Search", "/ ? :s use regex (\\d \\w \\b ( ) | * + ? {n,m}); :s replacement: $1 \\1 &"),
    ("", "EXPLORER (focused)"),
    ("↑/↓  Enter  ←/→", "Browse (preview), open, collapse/expand"),
    ("a / A / r / d / R / .", "New file / folder / rename / delete / refresh / hidden"),
    ("", "TERMINAL (focused)"),
    ("Ctrl+C (with selection)", "Copy, otherwise interrupt"),
    ("Ctrl+V", "Paste"),
    ("Shift+PgUp/PgDn", "Scroll back"),
];

fn help_rows() -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = HELP.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    rows.push((String::new(), "KEYBOARD PROTOCOL".into()));
    rows.push(("⌨ kitty / xterm".into(), "Ctrl+Shift+X is distinct from Ctrl+X (Ctrl+Shift+Z redo, ...)".into()));
    rows.push((
        "⌨ legacy (Terminal.app)".into(),
        "Ctrl+Shift+X arrives as Ctrl+X: Ctrl+L/R/Y/P open panels; redo = Ctrl+Y, Alt+Shift+Z or F4".into(),
    ));
    rows.push((
        "⌨ legacy+⇧".into(),
        "Same terminal, but cmx reads Shift from the OS: Ctrl+Shift+X works (Linux X11; macOS below)".into(),
    ));
    rows.push((
        "Terminal.app".into(),
        "Allow Terminal in Privacy & Security › Input Monitoring, reopen it (palette: Keyboard: Detect Ctrl+Shift)"
            .into(),
    ));
    rows.push((
        "VS Code terminal".into(),
        "Enable terminal.integrated.enableKittyKeyboardProtocol and sendKeybindingsToShell".into(),
    ));
    rows
}

fn draw_help(app: &mut App, buf: &mut Buffer) {
    let th = app.theme;
    let rows = help_rows();
    let w = app.width.saturating_sub(4).min(100);
    let h = app.height.saturating_sub(4).min(rows.len() as u16 + 2);
    let r = Rect::new((app.width - w) / 2, (app.height.saturating_sub(h)) / 2, w, h);
    draw_box(buf, r, th, "cmx — Keyboard shortcuts (Esc to close, ↑↓ scroll)");
    let inner = r.inset(2, 1, 2, 1);
    app.hits.help_box = Some(inner);
    let max_scroll = rows.len().saturating_sub(inner.h as usize);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let kw = 30u16.min(inner.w / 2);
    for (k, (key, desc)) in rows.iter().skip(app.help_scroll).take(inner.h as usize).enumerate() {
        let y = inner.y + k as u16;
        if key.is_empty() {
            buf.put_str(inner.x, y, desc, th.overlay().fg(th.overlay_match_fg).bold(), inner.right());
        } else {
            buf.put_str(inner.x + 1, y, key, th.overlay().fg(th.syn_function), inner.x + kw);
            buf.put_str(inner.x + kw + 1, y, desc, th.overlay(), inner.right());
        }
    }
    let _ = Color::Reset;
}

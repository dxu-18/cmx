//! Screen layout: explorer | (editor / terminal panel) | claude, plus the status bar.

use cmx_tui::Rect;

pub struct LayoutInput<'a> {
    pub width: u16,
    pub height: u16,
    pub show_tree: bool,
    pub tree_w: u16,
    pub show_claude: bool,
    pub claude_w: u16,
    pub show_panel: bool,
    pub panel_h: u16,
    /// Relative widths of the split terminals (sum ≈ 1.0).
    pub term_ratios: &'a [f32],
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub status: Rect,
    pub tree: Option<Rect>,
    pub tree_split: Option<Rect>,
    pub editor: Rect,
    pub tabbar: Rect,
    pub text: Rect,
    pub panel: Option<Rect>,
    pub panel_header: Option<Rect>,
    pub terms: Vec<Rect>,
    pub term_splits: Vec<Rect>,
    pub claude: Option<Rect>,
    pub claude_header: Option<Rect>,
    pub claude_body: Option<Rect>,
    pub claude_split: Option<Rect>,
}

pub const MIN_TREE: u16 = 12;
pub const MIN_CLAUDE: u16 = 24;
pub const MIN_CENTER: u16 = 20;

pub fn compute(i: &LayoutInput) -> Layout {
    let mut l = Layout::default();
    let w = i.width;
    let main_h = i.height.saturating_sub(1);
    l.status = Rect::new(0, i.height.saturating_sub(1), w, 1.min(i.height));
    let mut x0 = 0u16;
    let mut x1 = w;
    if i.show_tree && w > MIN_TREE + MIN_CENTER {
        let tw = i.tree_w.clamp(MIN_TREE, w.saturating_sub(MIN_CENTER + 1).max(MIN_TREE));
        l.tree = Some(Rect::new(0, 0, tw, main_h));
        l.tree_split = Some(Rect::new(tw, 0, 1, main_h));
        x0 = tw + 1;
    }
    if i.show_claude && x1.saturating_sub(x0) > MIN_CLAUDE + MIN_CENTER {
        let avail = x1 - x0;
        let cw = i.claude_w.clamp(MIN_CLAUDE, avail - MIN_CENTER - 1);
        let c = Rect::new(x1 - cw, 0, cw, main_h);
        l.claude = Some(c);
        l.claude_header = Some(c.row(0));
        l.claude_body = Some(c.inset(0, 1, 0, 0));
        l.claude_split = Some(Rect::new(x1 - cw - 1, 0, 1, main_h));
        x1 = x1 - cw - 1;
    }
    let cw = x1.saturating_sub(x0);
    let mut editor = Rect::new(x0, 0, cw, main_h);
    if i.show_panel && main_h >= 8 {
        let ph = i.panel_h.clamp(3, main_h - 4);
        let p = Rect::new(x0, main_h - ph, cw, ph);
        l.panel = Some(p);
        l.panel_header = Some(p.row(0));
        editor.h = main_h - ph;
        let body = p.inset(0, 1, 0, 0);
        let n = i.term_ratios.len().max(1);
        let usable = body.w.saturating_sub((n as u16).saturating_sub(1));
        let mut x = body.x;
        for k in 0..n {
            let last = k + 1 == n;
            let ww = if last {
                body.right().saturating_sub(x)
            } else {
                let r = i.term_ratios.get(k).copied().unwrap_or(1.0 / n as f32);
                ((usable as f32 * r).round() as u16).clamp(4.min(usable), usable)
            };
            l.terms.push(Rect::new(x, body.y, ww, body.h));
            x += ww;
            if !last {
                l.term_splits.push(Rect::new(x, body.y, 1, body.h));
                x += 1;
            }
        }
    }
    l.editor = editor;
    l.tabbar = editor.row(0);
    l.text = editor.inset(0, 1, 0, 0);
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_layout() {
        let l = compute(&LayoutInput {
            width: 120,
            height: 40,
            show_tree: true,
            tree_w: 25,
            show_claude: true,
            claude_w: 40,
            show_panel: true,
            panel_h: 10,
            term_ratios: &[0.5, 0.5],
        });
        assert_eq!(l.tree.unwrap(), Rect::new(0, 0, 25, 39));
        assert_eq!(l.claude.unwrap(), Rect::new(80, 0, 40, 39));
        assert_eq!(l.editor, Rect::new(26, 0, 53, 29));
        assert_eq!(l.panel.unwrap(), Rect::new(26, 29, 53, 10));
        assert_eq!(l.terms.len(), 2);
        assert_eq!(l.terms[0].w + l.terms[1].w + 1, 53);
        assert_eq!(l.status, Rect::new(0, 39, 120, 1));
    }

    #[test]
    fn tiny_screen_hides_sidebars() {
        let l = compute(&LayoutInput {
            width: 30,
            height: 6,
            show_tree: true,
            tree_w: 25,
            show_claude: true,
            claude_w: 40,
            show_panel: true,
            panel_h: 10,
            term_ratios: &[1.0],
        });
        assert!(l.tree.is_none());
        assert!(l.claude.is_none());
        assert!(l.panel.is_none());
        assert_eq!(l.editor.w, 30);
    }
}

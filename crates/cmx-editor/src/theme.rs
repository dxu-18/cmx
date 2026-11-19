//! Colors (a VS Code "Dark+"-inspired palette).

use cmx_tui::{Color, Style, rgb};

pub struct Theme {
    pub editor_bg: Color,
    pub editor_fg: Color,
    pub gutter_fg: Color,
    pub gutter_active_fg: Color,
    pub cursor_line_bg: Color,
    pub selection_bg: Color,
    pub search_bg: Color,
    pub search_cur_bg: Color,
    pub bracket_bg: Color,
    pub sidebar_bg: Color,
    pub sidebar_fg: Color,
    pub sidebar_dir_fg: Color,
    pub sidebar_sel_bg: Color,
    pub sidebar_sel_focus_bg: Color,
    pub header_bg: Color,
    pub header_fg: Color,
    pub header_focus_fg: Color,
    pub tabbar_bg: Color,
    pub tab_bg: Color,
    pub tab_fg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_dirty_fg: Color,
    pub split_fg: Color,
    pub split_focus_fg: Color,
    pub status_bg: Color,
    pub status_fg: Color,
    pub status_vim_normal_bg: Color,
    pub status_vim_insert_bg: Color,
    pub status_vim_visual_bg: Color,
    pub status_warn_fg: Color,
    pub status_err_fg: Color,
    pub overlay_bg: Color,
    pub overlay_fg: Color,
    pub overlay_border: Color,
    pub overlay_sel_bg: Color,
    pub overlay_match_fg: Color,
    pub dim_fg: Color,
    pub term_bg: Color,
    pub term_fg: Color,
    pub scrollbar: Color,
    // syntax
    pub syn_keyword: Color,
    pub syn_control: Color,
    pub syn_type: Color,
    pub syn_function: Color,
    pub syn_string: Color,
    pub syn_number: Color,
    pub syn_comment: Color,
    pub syn_constant: Color,
    pub syn_macro: Color,
    pub syn_operator: Color,
    pub syn_heading: Color,
    pub syn_variable: Color,
    pub syn_tag: Color,
    pub syn_math: Color,
}

pub const DARK: Theme = Theme {
    editor_bg: rgb(0x1e1e1e),
    editor_fg: rgb(0xd4d4d4),
    gutter_fg: rgb(0x6e7681),
    gutter_active_fg: rgb(0xc6c6c6),
    cursor_line_bg: rgb(0x262626),
    selection_bg: rgb(0x264f78),
    search_bg: rgb(0x613214),
    search_cur_bg: rgb(0x9e6a03),
    bracket_bg: rgb(0x3a3d41),
    sidebar_bg: rgb(0x252526),
    sidebar_fg: rgb(0xcccccc),
    sidebar_dir_fg: rgb(0xe8e8e8),
    sidebar_sel_bg: rgb(0x37373d),
    sidebar_sel_focus_bg: rgb(0x04395e),
    header_bg: rgb(0x252526),
    header_fg: rgb(0x9d9d9d),
    header_focus_fg: rgb(0xffffff),
    tabbar_bg: rgb(0x252526),
    tab_bg: rgb(0x2d2d2d),
    tab_fg: rgb(0x969696),
    tab_active_bg: rgb(0x1e1e1e),
    tab_active_fg: rgb(0xffffff),
    tab_dirty_fg: rgb(0xe2c08d),
    split_fg: rgb(0x3c3c3c),
    split_focus_fg: rgb(0x007acc),
    status_bg: rgb(0x007acc),
    status_fg: rgb(0xffffff),
    status_vim_normal_bg: rgb(0x16825d),
    status_vim_insert_bg: rgb(0x007acc),
    status_vim_visual_bg: rgb(0x8b4ec2),
    status_warn_fg: rgb(0xffd866),
    status_err_fg: rgb(0xffb4b4),
    overlay_bg: rgb(0x252526),
    overlay_fg: rgb(0xcccccc),
    overlay_border: rgb(0x454545),
    overlay_sel_bg: rgb(0x04395e),
    overlay_match_fg: rgb(0x2aaaff),
    dim_fg: rgb(0x808080),
    term_bg: rgb(0x181818),
    term_fg: rgb(0xcccccc),
    scrollbar: rgb(0x424242),
    syn_keyword: rgb(0x569cd6),
    syn_control: rgb(0xc586c0),
    syn_type: rgb(0x4ec9b0),
    syn_function: rgb(0xdcdcaa),
    syn_string: rgb(0xce9178),
    syn_number: rgb(0xb5cea8),
    syn_comment: rgb(0x6a9955),
    syn_constant: rgb(0x4fc1ff),
    syn_macro: rgb(0xc586c0),
    syn_operator: rgb(0xd4d4d4),
    syn_heading: rgb(0x569cd6),
    syn_variable: rgb(0x9cdcfe),
    syn_tag: rgb(0x569cd6),
    syn_math: rgb(0xdcdcaa),
};

impl Theme {
    pub fn editor(&self) -> Style {
        Style::new(self.editor_fg, self.editor_bg)
    }
    pub fn sidebar(&self) -> Style {
        Style::new(self.sidebar_fg, self.sidebar_bg)
    }
    pub fn status(&self) -> Style {
        Style::new(self.status_fg, self.status_bg)
    }
    pub fn overlay(&self) -> Style {
        Style::new(self.overlay_fg, self.overlay_bg)
    }
    pub fn term(&self) -> Style {
        Style::new(self.term_fg, self.term_bg)
    }
}

//! cmx-tui: the terminal UI toolkit behind the cmx editor (no external crates).
//!
//! * [`unicode`]   display widths + grapheme clusters
//! * [`style`]     colors / attributes
//! * [`buffer`]    off-screen cell buffer with wide-char invariants
//! * [`render`]    frame diffing renderer (truecolor or 256-color)
//! * [`input`]     input byte parser (keys, mouse, paste, focus, protocol replies)
//! * [`clipboard`] OSC 52 + system clipboard tools
//! * [`term`]      escape sequences to enter/leave the full-screen UI

pub mod buffer;
pub mod clipboard;
pub mod input;
pub mod render;
pub mod style;
pub mod unicode;

pub use buffer::{Buffer, Cell, Rect, Sym};
pub use input::{Event, InputParser, KeyCode, KeyEvent, Mods, MouseButton, MouseEvent, MouseKind};
pub use render::{CursorShape, CursorState, Renderer};
pub use style::{Color, Style, attr, rgb};

/// Escape sequences for entering / leaving the full-screen UI.
pub mod term {
    /// Alt screen, SGR any-event mouse (motion too, for hover feedback), bracketed paste, focus
    /// events, then the kitty keyboard query followed by DA1 (if the DA1 reply arrives first,
    /// kitty is unsupported).
    pub const ENTER: &[u8] =
        b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?2004h\x1b[?1004h\x1b[?u\x1b[c";
    /// Push kitty keyboard flags = 1 (disambiguate escape codes).
    pub const KITTY_PUSH: &[u8] = b"\x1b[>1u";
    pub const KITTY_POP: &[u8] = b"\x1b[<u";
    /// xterm modifyOtherKeys level 2 (fallback when kitty is unsupported).
    pub const MOK_ENABLE: &[u8] = b"\x1b[>4;2m";
    pub const MOK_DISABLE: &[u8] = b"\x1b[>4m";
    /// Undo everything in `ENTER` (and the cursor shape), restore the main screen.
    pub const LEAVE: &[u8] =
        b"\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?1049l";
    /// Mouse pointer shape (OSC 22, CSS names; kitty, Ghostty, foot, WezTerm). Terminals without
    /// support ignore it.
    pub fn pointer(shape: &str) -> Vec<u8> {
        format!("\x1b]22;{shape}\x1b\\").into_bytes()
    }
}

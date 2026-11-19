//! cmx-editor: the cmx terminal IDE (`cmx edit`, `cmx workspace`).
//! Architecture notes live in claude/EDITOR-DESIGN.md.

pub mod app;
pub mod document;
pub mod highlight;
pub mod history;
pub mod keymap;
pub mod layout;
pub mod palette;
pub mod prompt;
pub mod regex;
pub mod render;
pub mod search;
pub mod termpane;
pub mod text;
pub mod theme;
pub mod tree;
pub mod view;
pub mod vim;

pub use app::Options;

use cmx_sys::signals::SignalPipe;
use cmx_sys::{fd, ffi, tty};
use std::io;

fn truecolor_supported() -> bool {
    if let Ok(v) = std::env::var("CMX_TRUECOLOR") {
        return v != "0";
    }
    let tp = std::env::var("TERM_PROGRAM").unwrap_or_default();
    if tp == "Apple_Terminal" {
        return false;
    }
    let ct = std::env::var("COLORTERM").unwrap_or_default();
    if ct == "truecolor" || ct == "24bit" {
        return true;
    }
    matches!(tp.as_str(), "iTerm.app" | "vscode" | "WezTerm" | "ghostty" | "kitty" | "Hyper" | "Tabby" | "rio" | "cmx")
        || std::env::var("TERM")
            .map(|t| t.contains("kitty") || t.contains("direct") || t.contains("alacritty") || t.contains("ghostty"))
            .unwrap_or(false)
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = fd::write_all(1, cmx_tui::term::KITTY_POP);
        let _ = fd::write_all(1, cmx_tui::term::MOK_DISABLE);
        let _ = fd::write_all(1, cmx_tui::term::LEAVE);
        tty::restore_original();
        prev(info);
    }));
}

/// Run the editor until the user quits. Requires stdin/stdout to be a terminal.
pub fn run(opts: Options) -> io::Result<()> {
    if !tty::is_tty(0) || !tty::is_tty(1) {
        return Err(io::Error::other("the cmx editor needs an interactive terminal"));
    }
    let _ = std::env::set_current_dir(&opts.root);
    let raw = tty::RawMode::enable(0)?;
    let orig = *raw.original();
    install_panic_hook();
    let sig = SignalPipe::install(&[ffi::SIGWINCH, ffi::SIGCHLD])?;
    fd::write_all(1, cmx_tui::term::ENTER)?;
    let (w, h) = tty::window_size(1).unwrap_or((80, 24));
    let mut app = app::App::new(opts, w, h, Some(orig), truecolor_supported());
    let res = app.main_loop(&sig);
    app.shutdown();
    drop(app);
    drop(sig);
    drop(raw);
    res
}

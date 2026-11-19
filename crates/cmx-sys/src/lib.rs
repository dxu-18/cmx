//! cmx-sys: the minimal POSIX layer cmx needs, for macOS and Linux, with zero external crates.
//!
//! * [`tty`]     raw mode, window size, isatty
//! * [`fd`]      non-blocking / close-on-exec helpers, robust read/write
//! * [`poll`]    `poll(2)` wrapper
//! * [`signals`] self-pipe signal delivery (SIGWINCH, SIGCHLD, ...)
//! * [`pty`]     pseudo-terminal spawning for embedded shells

pub mod fd;
pub mod ffi;
pub mod keytap;
pub mod modkeys;
pub mod poll;
pub mod pty;
pub mod signals;
pub mod tty;

pub use ffi::{termios_buf as Termios, winsize as WinSize};

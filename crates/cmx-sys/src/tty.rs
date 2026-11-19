//! Controlling-terminal helpers: raw mode and window size.

use crate::ffi::{self, termios_buf};
use std::io;
use std::os::raw::c_int;
use std::sync::Mutex;

/// The original termios of the terminal we put into raw mode, kept globally so a panic hook can
/// restore the user's terminal even when the guard is never dropped (panic = "abort").
static ORIGINAL: Mutex<Option<(c_int, termios_buf)>> = Mutex::new(None);

pub fn is_tty(fd: c_int) -> bool {
    unsafe { ffi::isatty(fd) == 1 }
}

/// Terminal size as `(cols, rows)`.
pub fn window_size(fd: c_int) -> Option<(u16, u16)> {
    let mut ws = ffi::winsize::default();
    let r = unsafe { ffi::ioctl(fd, ffi::TIOCGWINSZ, &mut ws as *mut ffi::winsize) };
    if r == 0 && ws.ws_col > 0 && ws.ws_row > 0 { Some((ws.ws_col, ws.ws_row)) } else { None }
}

/// RAII guard: puts `fd` into raw mode, restores the original settings on drop.
pub struct RawMode {
    fd: c_int,
    orig: termios_buf,
}

impl RawMode {
    pub fn enable(fd: c_int) -> io::Result<RawMode> {
        let mut orig = termios_buf::zeroed();
        if unsafe { ffi::tcgetattr(fd, &mut orig) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = orig;
        unsafe { ffi::cfmakeraw(&mut raw) };
        if unsafe { ffi::tcsetattr(fd, ffi::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if let Ok(mut g) = ORIGINAL.lock() {
            *g = Some((fd, orig));
        }
        Ok(RawMode { fd, orig })
    }

    /// The cooked settings the terminal had before raw mode (handy for child PTYs).
    pub fn original(&self) -> &termios_buf {
        &self.orig
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe { ffi::tcsetattr(self.fd, ffi::TCSANOW, &self.orig) };
        if let Ok(mut g) = ORIGINAL.lock() {
            *g = None;
        }
    }
}

/// Restore the terminal from the global copy (for panic hooks). Never blocks: uses `try_lock`.
pub fn restore_original() {
    if let Ok(g) = ORIGINAL.try_lock()
        && let Some((fd, t)) = g.as_ref()
    {
        unsafe { ffi::tcsetattr(*fd, ffi::TCSANOW, t) };
    }
}

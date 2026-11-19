//! Raw libc declarations. Hand-written (no `libc` crate): see claude/CORE-DESIGN.md D-2.
//!
//! Only functions whose signatures are identical on macOS and Linux are declared here; layouts that
//! differ (notably `struct termios`) are kept opaque. Variadic functions (`ioctl`, `fcntl`, `open`)
//! MUST stay declared as variadic: Apple arm64 passes variadic arguments on the stack.
#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_short, c_ulong, c_void};

pub type pid_t = i32;
pub type ssize_t = isize;
pub type size_t = usize;

#[cfg(target_os = "macos")]
pub type nfds_t = std::os::raw::c_uint;
#[cfg(not(target_os = "macos"))]
pub type nfds_t = c_ulong;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct pollfd {
    pub fd: c_int,
    pub events: c_short,
    pub revents: c_short,
}

/// Opaque storage for `struct termios`, large enough for every supported libc
/// (glibc: 60 bytes, musl: 60, macOS: 72). We never read individual fields; we only
/// round-trip it through `tcgetattr` / `cfmakeraw` / `tcsetattr`.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct termios_buf {
    pub bytes: [u8; 256],
}

impl termios_buf {
    pub const fn zeroed() -> Self {
        termios_buf { bytes: [0; 256] }
    }
}

unsafe extern "C" {
    pub fn tcgetattr(fd: c_int, t: *mut termios_buf) -> c_int;
    pub fn tcsetattr(fd: c_int, optional_actions: c_int, t: *const termios_buf) -> c_int;
    pub fn cfmakeraw(t: *mut termios_buf);
    pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    pub fn poll(fds: *mut pollfd, nfds: nfds_t, timeout: c_int) -> c_int;
    pub fn isatty(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t;
    pub fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;
    pub fn close(fd: c_int) -> c_int;
    pub fn pipe(fds: *mut c_int) -> c_int;
    pub fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    pub fn fork() -> pid_t;
    pub fn setsid() -> pid_t;
    pub fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int;
    pub fn chdir(path: *const c_char) -> c_int;
    pub fn _exit(status: c_int) -> !;
    pub fn waitpid(pid: pid_t, status: *mut c_int, options: c_int) -> pid_t;
    pub fn kill(pid: pid_t, sig: c_int) -> c_int;
    pub fn signal(sig: c_int, handler: usize) -> usize;
    pub fn posix_openpt(flags: c_int) -> c_int;
    pub fn grantpt(fd: c_int) -> c_int;
    pub fn unlockpt(fd: c_int) -> c_int;
    pub fn ptsname(fd: c_int) -> *mut c_char;
    pub fn getpid() -> pid_t;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    #[link_name = "__error"]
    pub fn errno_location() -> *mut c_int;
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    #[link_name = "__errno_location"]
    pub fn errno_location() -> *mut c_int;
}

// ---- constants common to macOS and Linux -------------------------------------------------

pub const TCSANOW: c_int = 0;
pub const TCSAFLUSH: c_int = 2;

pub const F_GETFD: c_int = 1;
pub const F_SETFD: c_int = 2;
pub const F_GETFL: c_int = 3;
pub const F_SETFL: c_int = 4;
pub const FD_CLOEXEC: c_int = 1;

pub const POLLIN: c_short = 0x1;
pub const POLLOUT: c_short = 0x4;
pub const POLLERR: c_short = 0x8;
pub const POLLHUP: c_short = 0x10;
pub const POLLNVAL: c_short = 0x20;

pub const WNOHANG: c_int = 1;

pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;

pub const SIGHUP: c_int = 1;
pub const SIGINT: c_int = 2;
pub const SIGQUIT: c_int = 3;
pub const SIGKILL: c_int = 9;
pub const SIGPIPE: c_int = 13;
pub const SIGTERM: c_int = 15;
pub const SIGWINCH: c_int = 28;

pub const O_RDWR: c_int = 2;

pub const EINTR: c_int = 4;
pub const EIO: c_int = 5;

// ---- per-OS constants --------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod os {
    use std::os::raw::{c_int, c_ulong};
    pub const TIOCGWINSZ: c_ulong = 0x4008_7468;
    pub const TIOCSWINSZ: c_ulong = 0x8008_7467;
    pub const TIOCSCTTY: c_ulong = 0x2000_7461;
    pub const O_NOCTTY: c_int = 0x20000;
    pub const O_NONBLOCK: c_int = 0x4;
    pub const O_CLOEXEC: c_int = 0x0100_0000;
    pub const SIGCHLD: c_int = 20;
    pub const SIGTSTP: c_int = 18;
    pub const SIGCONT: c_int = 19;
    pub const EAGAIN: c_int = 35;
}

#[cfg(target_os = "linux")]
mod os {
    use std::os::raw::{c_int, c_ulong};
    pub const TIOCGWINSZ: c_ulong = 0x5413;
    pub const TIOCSWINSZ: c_ulong = 0x5414;
    pub const TIOCSCTTY: c_ulong = 0x540E;
    pub const O_NOCTTY: c_int = 0o400;
    pub const O_NONBLOCK: c_int = 0o4000;
    pub const O_CLOEXEC: c_int = 0o2000000;
    pub const SIGCHLD: c_int = 17;
    pub const SIGTSTP: c_int = 20;
    pub const SIGCONT: c_int = 18;
    pub const EAGAIN: c_int = 11;
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("cmx supports macOS and Linux only");

pub use os::*;

/// Read the calling thread's `errno`.
#[inline]
pub fn errno() -> c_int {
    unsafe { *errno_location() }
}

/// Overwrite the calling thread's `errno` (used by the signal handler to preserve it).
#[inline]
pub fn set_errno(v: c_int) {
    unsafe { *errno_location() = v }
}

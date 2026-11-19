//! Self-pipe signal delivery: the handler writes the signal number into a non-blocking pipe that
//! the event loop polls. Async-signal-safe (only `write(2)`, errno preserved).

use crate::fd::{self, RawFd};
use crate::ffi;
use std::io;
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicI32, Ordering};

static WRITE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(sig: c_int) {
    let saved = ffi::errno();
    let fd = WRITE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = sig as u8;
        unsafe { ffi::write(fd, &b as *const u8 as *const c_void, 1) };
    }
    ffi::set_errno(saved);
}

pub struct SignalPipe {
    read_fd: RawFd,
    write_fd: RawFd,
    signals: Vec<c_int>,
}

impl SignalPipe {
    /// Install handlers for `signals`. Only one `SignalPipe` should exist per process.
    pub fn install(signals: &[c_int]) -> io::Result<SignalPipe> {
        let mut fds = [0 as c_int; 2];
        if unsafe { ffi::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for &f in &fds {
            fd::set_nonblocking(f, true)?;
            fd::set_cloexec(f)?;
        }
        WRITE_FD.store(fds[1], Ordering::Relaxed);
        let handler = on_signal as extern "C" fn(c_int) as usize;
        for &s in signals {
            unsafe { ffi::signal(s, handler) };
        }
        Ok(SignalPipe { read_fd: fds[0], write_fd: fds[1], signals: signals.to_vec() })
    }

    pub fn fd(&self) -> RawFd {
        self.read_fd
    }

    /// Drain pending signal numbers (deduplicated, in arrival order).
    pub fn drain(&self) -> Vec<c_int> {
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        while let Ok(n) = fd::read(self.read_fd, &mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                let s = b as c_int;
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        out
    }
}

impl Drop for SignalPipe {
    fn drop(&mut self) {
        for &s in &self.signals {
            unsafe { ffi::signal(s, ffi::SIG_DFL) };
        }
        WRITE_FD.store(-1, Ordering::Relaxed);
        fd::close(self.read_fd);
        fd::close(self.write_fd);
    }
}

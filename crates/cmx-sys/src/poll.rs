//! `poll(2)` wrapper.

use crate::ffi;
use std::io;

pub use ffi::pollfd as PollFd;
pub use ffi::{POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT};

impl PollFd {
    pub fn new(fd: i32, events: i16) -> PollFd {
        PollFd { fd, events, revents: 0 }
    }
    #[inline]
    pub fn readable(&self) -> bool {
        self.revents & (POLLIN | POLLHUP | POLLERR | POLLNVAL) != 0
    }
    #[inline]
    pub fn writable(&self) -> bool {
        self.revents & POLLOUT != 0
    }
}

/// Wait for events. `timeout_ms < 0` waits forever. Returns `Ok(0)` on timeout *or* when
/// interrupted by a signal (so callers can drain their signal pipe).
pub fn poll(fds: &mut [PollFd], timeout_ms: i32) -> io::Result<usize> {
    let n = unsafe { ffi::poll(fds.as_mut_ptr(), fds.len() as ffi::nfds_t, timeout_ms) };
    if n >= 0 {
        return Ok(n as usize);
    }
    let e = io::Error::last_os_error();
    if e.kind() == io::ErrorKind::Interrupted {
        for f in fds.iter_mut() {
            f.revents = 0;
        }
        return Ok(0);
    }
    Err(e)
}

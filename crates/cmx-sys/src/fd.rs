//! File-descriptor helpers.

use crate::ffi;
use std::io;
use std::os::raw::{c_int, c_void};

pub type RawFd = c_int;

pub fn set_nonblocking(fd: RawFd, on: bool) -> io::Result<()> {
    unsafe {
        let flags = ffi::fcntl(fd, ffi::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let nf = if on { flags | ffi::O_NONBLOCK } else { flags & !ffi::O_NONBLOCK };
        if nf != flags && ffi::fcntl(fd, ffi::F_SETFL, nf) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn set_cloexec(fd: RawFd) -> io::Result<()> {
    unsafe {
        let flags = ffi::fcntl(fd, ffi::F_GETFD);
        if flags < 0 || ffi::fcntl(fd, ffi::F_SETFD, flags | ffi::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// `read(2)`; returns `Ok(0)` on EOF. `EINTR` is retried. `EAGAIN` surfaces as `WouldBlock`.
pub fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        let n = unsafe { ffi::read(fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(e);
    }
}

/// `write(2)`; `EINTR` is retried. May write fewer bytes than given.
pub fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    loop {
        let n = unsafe { ffi::write(fd, buf.as_ptr() as *const c_void, buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(e);
    }
}

/// Write the whole buffer to a (possibly non-blocking) fd, waiting with `poll` on `EAGAIN`.
pub fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        match write(fd, buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0")),
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let mut p = [ffi::pollfd { fd, events: ffi::POLLOUT, revents: 0 }];
                unsafe { ffi::poll(p.as_mut_ptr(), 1, 100) };
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn close(fd: RawFd) {
    if fd >= 0 {
        unsafe { ffi::close(fd) };
    }
}

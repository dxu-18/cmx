//! Pseudo-terminal process spawning (for the embedded shell / claude panes).
//!
//! Uses only POSIX calls present in both glibc and libSystem: posix_openpt, grantpt, unlockpt,
//! ptsname, fork, setsid, ioctl(TIOCSCTTY), dup2, execve.

use crate::fd::{self, RawFd};
use crate::ffi::{self, pid_t, termios_buf, winsize};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::raw::{c_char, c_int};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// What to run inside the pty.
#[derive(Clone, Debug, Default)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Environment overrides applied on top of the current environment.
    pub env: Vec<(String, String)>,
    /// Variables removed from the inherited environment.
    pub env_remove: Vec<String>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        CommandSpec { program: program.into(), ..Default::default() }
    }
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }
    pub fn cwd(mut self, p: impl Into<PathBuf>) -> Self {
        self.cwd = Some(p.into());
        self
    }
    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(p) {
        Ok(m) => m.is_file() && m.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

/// Resolve `program` against `$PATH` (or return it as-is when it contains a `/`).
pub fn find_program(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return if is_executable(&p) { Some(p) } else { None };
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(program);
        if is_executable(&cand) {
            return Some(cand);
        }
    }
    None
}

pub struct Pty {
    master: RawFd,
    pid: pid_t,
    status: Option<i32>,
}

fn cstr(b: &[u8]) -> io::Result<CString> {
    CString::new(b).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL byte in argument"))
}

impl Pty {
    /// Spawn `spec` attached to a new pty of `cols`×`rows`. `termios`, when given, is applied to
    /// the slave side (pass the user's original cooked settings so the shell behaves normally).
    pub fn spawn(spec: &CommandSpec, cols: u16, rows: u16, termios: Option<&termios_buf>) -> io::Result<Pty> {
        let path = find_program(&spec.program)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{}: command not found", spec.program)))?;

        // ---- everything the child needs is prepared *before* fork (no allocation after fork) ----
        let c_path = cstr(path.as_os_str().as_bytes())?;
        let argv0 = Path::new(&spec.program)
            .file_name()
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_else(|| spec.program.as_bytes().to_vec());
        let mut argv: Vec<CString> = vec![cstr(&argv0)?];
        for a in &spec.args {
            argv.push(cstr(a.as_bytes())?);
        }
        let argv_ptrs: Vec<*const c_char> =
            argv.iter().map(|c| c.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();

        let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(k, _)| {
                let ks = k.to_string_lossy();
                !spec.env_remove.iter().any(|r| r == ks.as_ref()) && !spec.env.iter().any(|(ok, _)| ok == ks.as_ref())
            })
            .collect();
        for (k, v) in &spec.env {
            env.push((OsString::from(k), OsString::from(v)));
        }
        let mut env_c: Vec<CString> = Vec::with_capacity(env.len());
        for (k, v) in env {
            let mut kv = k.into_vec();
            kv.push(b'=');
            kv.extend_from_slice(OsStr::as_bytes(&v));
            if let Ok(c) = CString::new(kv) {
                env_c.push(c);
            }
        }
        let env_ptrs: Vec<*const c_char> =
            env_c.iter().map(|c| c.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();
        let c_cwd = match &spec.cwd {
            Some(d) => Some(cstr(d.as_os_str().as_bytes())?),
            None => None,
        };

        // ---- pty pair ----
        let master = unsafe { ffi::posix_openpt(ffi::O_RDWR | ffi::O_NOCTTY) };
        if master < 0 {
            return Err(io::Error::last_os_error());
        }
        let fail = |m: RawFd| {
            let e = io::Error::last_os_error();
            fd::close(m);
            e
        };
        if unsafe { ffi::grantpt(master) } != 0 || unsafe { ffi::unlockpt(master) } != 0 {
            return Err(fail(master));
        }
        let name_ptr = unsafe { ffi::ptsname(master) };
        if name_ptr.is_null() {
            return Err(fail(master));
        }
        let slave_name: CString = unsafe { CStr::from_ptr(name_ptr) }.to_owned();
        fd::set_cloexec(master)?;
        let ws = winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 };
        let tio: Option<termios_buf> = termios.copied();

        let pid = unsafe { ffi::fork() };
        if pid < 0 {
            return Err(fail(master));
        }
        if pid == 0 {
            // ---- child: async-signal-safe calls only ----
            unsafe {
                ffi::setsid();
                let s = ffi::open(slave_name.as_ptr(), ffi::O_RDWR);
                if s < 0 {
                    ffi::_exit(126);
                }
                ffi::ioctl(s, ffi::TIOCSCTTY, 0 as c_int);
                if let Some(t) = tio.as_ref() {
                    ffi::tcsetattr(s, ffi::TCSANOW, t);
                }
                ffi::ioctl(s, ffi::TIOCSWINSZ, &ws as *const winsize);
                ffi::dup2(s, 0);
                ffi::dup2(s, 1);
                ffi::dup2(s, 2);
                if s > 2 {
                    ffi::close(s);
                }
                ffi::close(master);
                if let Some(c) = c_cwd.as_ref() {
                    ffi::chdir(c.as_ptr());
                }
                // The Rust runtime ignores SIGPIPE; ignored dispositions survive exec.
                ffi::signal(ffi::SIGPIPE, ffi::SIG_DFL);
                ffi::signal(ffi::SIGINT, ffi::SIG_DFL);
                ffi::signal(ffi::SIGQUIT, ffi::SIG_DFL);
                ffi::execve(c_path.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr());
                ffi::_exit(127);
            }
        }

        // ---- parent ----
        fd::set_nonblocking(master, true)?;
        Ok(Pty { master, pid, status: None })
    }

    pub fn fd(&self) -> RawFd {
        self.master
    }

    pub fn pid(&self) -> pid_t {
        self.pid
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        fd::read(self.master, buf)
    }

    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        fd::write(self.master, data)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 };
        if unsafe { ffi::ioctl(self.master, ffi::TIOCSWINSZ, &ws as *const winsize) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Non-blocking reap. Returns the exit status once the child has exited.
    pub fn try_wait(&mut self) -> Option<i32> {
        if self.status.is_some() {
            return self.status;
        }
        let mut st: c_int = 0;
        let r = unsafe { ffi::waitpid(self.pid, &mut st, ffi::WNOHANG) };
        if r == self.pid {
            // WIFEXITED: (st & 0x7f) == 0 → WEXITSTATUS = (st >> 8) & 0xff; else 128 + signal
            let code = if st & 0x7f == 0 { (st >> 8) & 0xff } else { 128 + (st & 0x7f) };
            self.status = Some(code);
        } else if r < 0 {
            // ECHILD: someone else reaped it; treat as exited.
            self.status = Some(-1);
        }
        self.status
    }

    pub fn signal(&self, sig: c_int) {
        if self.status.is_none() {
            unsafe { ffi::kill(self.pid, sig) };
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        fd::close(self.master);
        if self.try_wait().is_none() {
            self.signal(ffi::SIGHUP);
            // Give it a moment, then make sure it doesn't linger as a zombie.
            for _ in 0..20 {
                if self.try_wait().is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            self.signal(ffi::SIGKILL);
            let mut st = 0;
            unsafe { ffi::waitpid(self.pid, &mut st, 0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_echo_and_read() {
        let spec = CommandSpec::new("/bin/sh").arg("-c").arg("printf 'hi %s' \"$CMX_T\"").env("CMX_T", "there");
        let mut pty = Pty::spawn(&spec, 80, 24, None).expect("spawn");
        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 5 {
            let mut p = [crate::poll::PollFd::new(pty.fd(), crate::poll::POLLIN)];
            crate::poll::poll(&mut p, 50).unwrap();
            match pty.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if pty.try_wait().is_some() && !p[0].readable() {
                        break;
                    }
                }
                Err(_) => break, // EIO when the slave closes (Linux)
            }
        }
        assert!(String::from_utf8_lossy(&out).contains("hi there"), "{:?}", String::from_utf8_lossy(&out));
    }

    #[test]
    fn missing_program() {
        let spec = CommandSpec::new("definitely-not-a-real-program-cmx");
        assert!(Pty::spawn(&spec, 80, 24, None).is_err());
    }
}

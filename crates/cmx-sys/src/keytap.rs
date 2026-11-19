//! macOS: see Ctrl+Shift+letter chords that the terminal app drops.
//!
//! Terminal.app sends *nothing* for Ctrl+Shift+letter (verified byte by byte; Ctrl+letter arrives
//! as the control byte). A listen-only Quartz event tap — what the Input Monitoring permission
//! allows — still sees the key, so cmx learns about the chord directly from the window server.
//! Each chord aimed at one of cmx's ancestor processes (the terminal app) is written to a pipe as
//! the lowercase character; the editor reads it as Ctrl+Shift+<char> while its tab has focus
//! (focus reporting), so typing in another Terminal tab or app is ignored.
//!
//! Silencing: Terminal.app beeps at a key it can't use. When the terminal also has the
//! Accessibility permission, the tap is an active one and *consumes* the chords it reports (only
//! Ctrl+Shift+letter, only aimed at cmx's Terminal, only while cmx's tab has focus), so Terminal
//! never sees them and doesn't beep. Without Accessibility it falls back to listening only (the
//! chords still work; Terminal still beeps). Nothing is ever injected or rewritten.

#[cfg(target_os = "macos")]
pub use imp::{set_focused, start};

/// How the tap runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Consumes the chords it reports (no beep from the terminal).
    Consume,
    /// Only observes them.
    Listen,
}

/// Not available on this platform.
#[cfg(not(target_os = "macos"))]
pub fn start() -> Result<(std::os::fd::RawFd, Mode), String> {
    Err("the key tap exists on macOS only".into())
}

#[cfg(not(target_os = "macos"))]
pub fn set_focused(_focused: bool) {}

#[cfg(target_os = "macos")]
mod imp {
    use super::Mode;
    use crate::fd;
    use std::ffi::c_void;
    use std::os::fd::RawFd;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

    type Ref = *mut c_void;
    type Callback = extern "C" fn(proxy: Ref, kind: u32, event: Ref, user: *mut c_void) -> Ref;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, cb: Callback, user: *mut c_void) -> Ref;
        fn CGEventTapEnable(tap: Ref, enable: bool);
        fn CGEventGetFlags(event: Ref) -> u64;
        fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
        fn CGEventKeyboardGetUnicodeString(event: Ref, max: usize, actual: *mut usize, buf: *mut u16);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFMachPortCreateRunLoopSource(alloc: Ref, port: Ref, order: isize) -> Ref;
        fn CFRunLoopGetCurrent() -> Ref;
        fn CFRunLoopAddSource(rl: Ref, source: Ref, mode: Ref);
        fn CFRunLoopRun();
        static kCFRunLoopCommonModes: Ref;
    }

    /// kCGAnnotatedSessionEventTap: events carry their target process.
    const ANNOTATED_SESSION: u32 = 2;
    const HEAD_INSERT: u32 = 0;
    const LISTEN_ONLY: u32 = 1;
    /// kCGEventTapOptionDefault: an active tap that may drop events (needs Accessibility).
    const ACTIVE: u32 = 0;
    const KEY_DOWN: u32 = 10;
    const DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const DISABLED_BY_USER: u32 = 0xFFFF_FFFF;
    const FIELD_KEYCODE: u32 = 9;
    const FIELD_TARGET_PID: u32 = 40;
    const FLAG_SHIFT: u64 = 0x0002_0000;
    const FLAG_CTRL: u64 = 0x0004_0000;
    const FLAG_ALT: u64 = 0x0008_0000;
    const FLAG_CMD: u64 = 0x0010_0000;

    static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static PIPE: AtomicI32 = AtomicI32::new(-1);
    static FOCUSED: AtomicBool = AtomicBool::new(true);
    static CONSUME: AtomicBool = AtomicBool::new(false);

    /// cmx's terminal tab gained / lost focus (focus reporting): chords count only while focused.
    pub fn set_focused(focused: bool) {
        FOCUSED.store(focused, Ordering::Relaxed);
    }
    static ANCESTORS: std::sync::OnceLock<Vec<i64>> = std::sync::OnceLock::new();

    /// US-ANSI key codes → characters (fallback when the event has no character).
    fn keycode_char(code: i64) -> Option<char> {
        const MAP: &[(i64, char)] = &[
            (0, 'a'),
            (1, 's'),
            (2, 'd'),
            (3, 'f'),
            (4, 'h'),
            (5, 'g'),
            (6, 'z'),
            (7, 'x'),
            (8, 'c'),
            (9, 'v'),
            (11, 'b'),
            (12, 'q'),
            (13, 'w'),
            (14, 'e'),
            (15, 'r'),
            (16, 'y'),
            (17, 't'),
            (31, 'o'),
            (32, 'u'),
            (34, 'i'),
            (35, 'p'),
            (37, 'l'),
            (38, 'j'),
            (40, 'k'),
            (45, 'n'),
            (46, 'm'),
        ];
        MAP.iter().find(|(c, _)| *c == code).map(|(_, ch)| *ch)
    }

    extern "C" fn callback(_proxy: Ref, kind: u32, event: Ref, _user: *mut c_void) -> Ref {
        if kind == DISABLED_BY_TIMEOUT || kind == DISABLED_BY_USER {
            let tap = TAP.load(Ordering::Relaxed);
            if !tap.is_null() {
                // SAFETY: re-enabling our own tap.
                unsafe { CGEventTapEnable(tap, true) };
            }
            return event;
        }
        if kind != KEY_DOWN || event.is_null() {
            return event;
        }
        // SAFETY: reading fields of the event the window server passed us.
        let (flags, target, code) = unsafe {
            (
                CGEventGetFlags(event),
                CGEventGetIntegerValueField(event, FIELD_TARGET_PID),
                CGEventGetIntegerValueField(event, FIELD_KEYCODE),
            )
        };
        let chord = flags & FLAG_CTRL != 0 && flags & FLAG_SHIFT != 0 && flags & (FLAG_CMD | FLAG_ALT) == 0;
        let ours = ANCESTORS.get().is_some_and(|a| a.contains(&target));
        if !chord || !ours || !FOCUSED.load(Ordering::Relaxed) {
            return event;
        }
        // the character this key makes on the user's layout (a control char or the letter)
        let mut buf = [0u16; 4];
        let mut n = 0usize;
        // SAFETY: a 4-unit buffer and its length.
        unsafe { CGEventKeyboardGetUnicodeString(event, buf.len(), &mut n, buf.as_mut_ptr()) };
        let from_event = (n == 1).then(|| buf[0]).and_then(|u| match u {
            1..=26 => char::from_u32(u as u32 + 0x60),
            0x41..=0x5a | 0x61..=0x7a => char::from_u32(u as u32).map(|c| c.to_ascii_lowercase()),
            _ => None,
        });
        if let Some(c) = from_event.or_else(|| keycode_char(code)) {
            let fd = PIPE.load(Ordering::Relaxed);
            if fd >= 0 {
                let _ = fd::write_all(fd, &[c as u8]);
            }
            if CONSUME.load(Ordering::Relaxed) {
                return std::ptr::null_mut(); // handled by cmx: Terminal never sees it (no beep)
            }
        }
        event
    }

    /// The Terminal.app process this cmx runs under (walking up from our parent), if any. Under
    /// tmux or ssh the chain doesn't reach it, and the tap stays off.
    fn terminal_app() -> Option<i64> {
        let mut pid = std::process::id() as i64;
        for _ in 0..16 {
            let out =
                std::process::Command::new("/bin/ps").args(["-o", "ppid=,comm=", "-p", &pid.to_string()]).output();
            let out = String::from_utf8_lossy(&out.ok()?.stdout).to_string();
            let line = out.trim();
            let (ppid, _) = line.split_once(char::is_whitespace)?;
            let ppid: i64 = ppid.trim().parse().ok()?;
            if ppid <= 1 {
                return None;
            }
            let parent = std::process::Command::new("/bin/ps").args(["-o", "comm=", "-p", &ppid.to_string()]).output();
            let comm = String::from_utf8_lossy(&parent.ok()?.stdout).trim().to_string();
            if comm.ends_with("Terminal.app/Contents/MacOS/Terminal") {
                return Some(ppid);
            }
            pid = ppid;
        }
        None
    }

    /// Start the tap on its own run-loop thread. Returns the read end of the chord pipe.
    /// Fails without the Input Monitoring permission (no tap can be created).
    pub fn start() -> Result<(RawFd, Mode), String> {
        let mut fds = [0i32; 2];
        // SAFETY: pipe(2) into a 2-int array.
        if unsafe { crate::ffi::pipe(fds.as_mut_ptr()) } != 0 {
            return Err("pipe failed".into());
        }
        let (rd, wr) = (fds[0], fds[1]);
        let _ = fd::set_nonblocking(rd, true);
        let _ = fd::set_cloexec(rd);
        let _ = fd::set_cloexec(wr);
        let terminal = terminal_app().ok_or("not running inside Terminal.app")?;
        let _ = ANCESTORS.set(vec![terminal]);
        PIPE.store(wr, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("cmx-keytap".into())
            .spawn(move || {
                // SAFETY: standard Quartz event-tap setup on this thread's run loop.
                unsafe {
                    // an active tap (Accessibility) can swallow the chords; else listen only
                    let mut mode = Mode::Consume;
                    let mut tap = CGEventTapCreate(
                        ANNOTATED_SESSION,
                        HEAD_INSERT,
                        ACTIVE,
                        1 << KEY_DOWN,
                        callback,
                        std::ptr::null_mut(),
                    );
                    if tap.is_null() {
                        mode = Mode::Listen;
                        tap = CGEventTapCreate(
                            ANNOTATED_SESSION,
                            HEAD_INSERT,
                            LISTEN_ONLY,
                            1 << KEY_DOWN,
                            callback,
                            std::ptr::null_mut(),
                        );
                    }
                    if tap.is_null() {
                        let _ = tx.send(None);
                        return;
                    }
                    CONSUME.store(mode == Mode::Consume, Ordering::Relaxed);
                    TAP.store(tap, Ordering::Relaxed);
                    let src = CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0);
                    CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
                    CGEventTapEnable(tap, true);
                    let _ = tx.send(Some(mode));
                    CFRunLoopRun();
                }
            })
            .map_err(|e| format!("thread: {e}"))?;
        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Some(mode)) => Ok((rd, mode)),
            _ => {
                PIPE.store(-1, Ordering::Relaxed);
                Err("no event tap (Input Monitoring not granted to this terminal)".into())
            }
        }
    }
}

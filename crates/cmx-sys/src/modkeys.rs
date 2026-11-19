//! Is Shift held right now? Used to tell Ctrl+Shift+letter from Ctrl+letter on terminals that send
//! both as the same control byte (macOS Terminal.app, many Linux terminals): when such a key
//! arrives, the user is still holding the modifiers, so the OS's current modifier state decides.
//!
//! * macOS: `CGEventSourceFlagsState` (CoreGraphics). Reading the keyboard state of another app
//!   needs the **Input Monitoring** privacy permission for the terminal app (TCC "ListenEvent");
//!   without it macOS reports no modifiers at all, so the probe returns `None` unless
//!   `CGPreflightListenEventAccess()` says access is granted. [`request_access`] asks macOS to show
//!   its permission prompt; the terminal must be reopened after the user allows it.
//! * Linux: `XQueryKeymap` through a runtime `dlopen("libX11.so.6")` when an X server is reachable
//!   (`$DISPLAY`); nothing is linked at build time. Native Wayland sessions have no global key
//!   state for other programs, so this returns `None` there.
//!
//! Over SSH the keyboard isn't on this machine, so the probe is off (`None`). `CMX_MODPROBE=0`
//! turns it off too.

use std::sync::OnceLock;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let ssh = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
        let off = std::env::var("CMX_MODPROBE").is_ok_and(|v| v == "0");
        !ssh && !off
    })
}

/// `Some(true)` if Shift is held now, `Some(false)` if not, `None` if this can't be known here.
pub fn shift_held() -> Option<bool> {
    if !enabled() {
        return None;
    }
    imp::shift_held()
}

/// The probe could work here but needs a permission the user hasn't granted (macOS Input
/// Monitoring for the terminal app).
pub fn needs_permission() -> bool {
    enabled() && imp::needs_permission()
}

/// Ask the OS to show its permission prompt (macOS: Input Monitoring for the terminal app; shown
/// once, macOS remembers the answer). Returns whether access is already granted.
pub fn request_access() -> bool {
    imp::request_access()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::OnceLock;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }
    /// kCGEventSourceStateCombinedSessionState / kCGEventSourceStateHIDSystemState
    const COMBINED_SESSION: i32 = 0;
    const HID_SYSTEM: i32 = 1;
    /// kCGEventFlagMaskShift
    const SHIFT: u64 = 0x0002_0000;

    /// Input Monitoring granted to the responsible app (the terminal). TCC changes only reach
    /// processes started afterwards, so checking once per run is enough.
    fn granted() -> bool {
        static G: OnceLock<bool> = OnceLock::new();
        // SAFETY: a TCC preflight query with no side effects.
        *G.get_or_init(|| unsafe { CGPreflightListenEventAccess() })
    }

    pub fn shift_held() -> Option<bool> {
        if !granted() {
            return None; // without Input Monitoring macOS reports no modifiers: don't trust it
        }
        // SAFETY: pure queries of the window server's modifier state.
        let flags = unsafe { CGEventSourceFlagsState(HID_SYSTEM) | CGEventSourceFlagsState(COMBINED_SESSION) };
        Some(flags & SHIFT != 0)
    }

    pub fn needs_permission() -> bool {
        !granted()
    }

    pub fn request_access() -> bool {
        // SAFETY: shows the system prompt the first time; afterwards returns the stored answer.
        granted() || unsafe { CGRequestListenEventAccess() }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{c_char, c_int, c_ulong, c_void};
    use std::sync::OnceLock;

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;

    type OpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type QueryKeymap = unsafe extern "C" fn(*mut c_void, *mut c_char) -> c_int;
    type KeysymToKeycode = unsafe extern "C" fn(*mut c_void, c_ulong) -> u8;

    struct X11 {
        display: usize,
        query: QueryKeymap,
        shift_codes: [u8; 2],
    }

    fn x11() -> Option<&'static X11> {
        static X: OnceLock<Option<X11>> = OnceLock::new();
        X.get_or_init(|| {
            std::env::var_os("DISPLAY")?;
            // SAFETY: dlopen/dlsym with NUL-terminated names; the symbols have the Xlib signatures.
            unsafe {
                let h = dlopen(c"libX11.so.6".as_ptr(), RTLD_NOW);
                if h.is_null() {
                    return None;
                }
                let sym = |n: &std::ffi::CStr| {
                    let p = dlsym(h, n.as_ptr());
                    (!p.is_null()).then_some(p)
                };
                let open: OpenDisplay = std::mem::transmute(sym(c"XOpenDisplay")?);
                let query: QueryKeymap = std::mem::transmute(sym(c"XQueryKeymap")?);
                let k2c: KeysymToKeycode = std::mem::transmute(sym(c"XKeysymToKeycode")?);
                let d = open(std::ptr::null());
                if d.is_null() {
                    return None;
                }
                // XK_Shift_L, XK_Shift_R
                let shift_codes = [k2c(d, 0xffe1), k2c(d, 0xffe2)];
                Some(X11 { display: d as usize, query, shift_codes })
            }
        })
        .as_ref()
    }

    pub fn needs_permission() -> bool {
        false
    }

    pub fn request_access() -> bool {
        x11().is_some()
    }

    pub fn shift_held() -> Option<bool> {
        let x = x11()?;
        let mut keys = [0u8; 32];
        // SAFETY: a 32-byte key vector, as XQueryKeymap requires; the display stays open.
        unsafe { (x.query)(x.display as *mut c_void, keys.as_mut_ptr().cast::<c_char>()) };
        let down = |code: u8| code != 0 && keys[(code / 8) as usize] & (1 << (code % 8)) != 0;
        Some(x.shift_codes.iter().any(|&c| down(c)))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    pub fn shift_held() -> Option<bool> {
        None
    }
    pub fn needs_permission() -> bool {
        false
    }
    pub fn request_access() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    /// Run with a key held: `CMX_EXPECT_SHIFT=1 cargo test -p cmx-sys -- --ignored shift_held_now`
    /// (verified on Linux with Xvfb + `xdotool keydown shift`).
    #[test]
    #[ignore = "needs a held Shift key (xdotool keydown shift under Xvfb, or a person)"]
    fn shift_held_now() {
        let want = std::env::var("CMX_EXPECT_SHIFT").is_ok_and(|v| v == "1");
        assert_eq!(super::shift_held(), Some(want));
    }

    #[test]
    fn probe_does_not_crash() {
        // no keys are held while tests run
        let r = super::shift_held();
        assert!(r.is_none() || r == Some(false), "{r:?}");
    }
}

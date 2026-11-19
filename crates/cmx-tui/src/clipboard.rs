//! Clipboard: internal register + OSC 52 (works over SSH, in kitty/WezTerm/iTerm2/...) + the
//! platform clipboard tool when available (pbcopy/pbpaste, wl-copy/wl-paste, xclip, xsel).

use std::io::Write;
use std::process::{Command, Stdio};

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in s.as_bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Mac,
    Wayland,
    Xclip,
    Xsel,
    None,
}

fn have(cmd: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|d| d.join(cmd).is_file())
}

fn detect() -> Tool {
    if cfg!(target_os = "macos") && have("pbcopy") {
        return Tool::Mac;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && have("wl-copy") {
        return Tool::Wayland;
    }
    if std::env::var_os("DISPLAY").is_some() {
        if have("xclip") {
            return Tool::Xclip;
        }
        if have("xsel") {
            return Tool::Xsel;
        }
    }
    Tool::None
}

pub struct Clipboard {
    internal: String,
    tool: Tool,
    /// Emit OSC 52 to the host terminal on copy.
    pub osc52: bool,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard {
    pub fn new() -> Clipboard {
        Clipboard { internal: String::new(), tool: detect(), osc52: true }
    }

    /// Copy text. OSC 52 bytes (if enabled) are appended to `term_out` for the caller to write.
    pub fn copy(&mut self, text: &str, term_out: &mut Vec<u8>) {
        self.internal = text.to_string();
        if self.osc52 && text.len() < 1_000_000 {
            term_out.extend_from_slice(b"\x1b]52;c;");
            term_out.extend_from_slice(base64_encode(text.as_bytes()).as_bytes());
            term_out.push(0x07);
        }
        let (cmd, args): (&str, &[&str]) = match self.tool {
            Tool::Mac => ("pbcopy", &[]),
            Tool::Wayland => ("wl-copy", &[]),
            Tool::Xclip => ("xclip", &["-selection", "clipboard", "-in"]),
            Tool::Xsel => ("xsel", &["--clipboard", "--input"]),
            Tool::None => return,
        };
        if let Ok(mut child) =
            Command::new(cmd).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }

    /// Paste: the system clipboard when a tool exists, else the internal register.
    pub fn paste(&mut self) -> String {
        let (cmd, args): (&str, &[&str]) = match self.tool {
            Tool::Mac => ("pbpaste", &[]),
            Tool::Wayland => ("wl-paste", &["--no-newline"]),
            Tool::Xclip => ("xclip", &["-selection", "clipboard", "-out"]),
            Tool::Xsel => ("xsel", &["--clipboard", "--output"]),
            Tool::None => return self.internal.clone(),
        };
        match Command::new(cmd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output() {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n");
                if s.is_empty() { self.internal.clone() } else { s }
            }
            _ => self.internal.clone(),
        }
    }

    /// The last text copied through this clipboard.
    pub fn internal(&self) -> &str {
        &self.internal
    }

    /// Remember text without touching the system clipboard (vim deletes).
    pub fn set_internal(&mut self, text: String) {
        self.internal = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn b64_roundtrip() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar", "héllo wörld ✓"] {
            let e = base64_encode(s.as_bytes());
            assert_eq!(base64_decode(&e).unwrap(), s.as_bytes());
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }
}

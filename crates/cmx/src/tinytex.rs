//! Bundled TinyTeX (vendor/tinytex, see its ORIGIN.md): `cmx stex compile` uses it instead of a
//! system TeX installation. The archive for this platform is unpacked once, next to the archive
//! (or under `~/.cmx/tinytex` when the vendor directory is read-only), then reused.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const VERSION: &str = "v2026.11-cmx1";

/// (archive platform tag, TeX Live bin directory name).
fn platform() -> Option<(&'static str, &'static str)> {
    if cfg!(target_os = "macos") {
        Some(("darwin", "universal-darwin"))
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(("linux-x86_64", "x86_64-linux"))
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some(("linux-arm64", "aarch64-linux"))
    } else {
        None
    }
}

/// Candidate `vendor/tinytex` directories: `$CMX_VENDOR/tinytex`, the source checkout cmx was
/// built from, then paths relative to the executable (`target/release/cmx` → `vendor/`).
fn vendor_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(d) = std::env::var("CMX_VENDOR")
        && !d.is_empty()
    {
        v.push(PathBuf::from(d).join("tinytex"));
    }
    v.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor/tinytex")));
    if let Ok(exe) = std::env::current_exe() {
        for up in ["../vendor", "../../vendor", "../../../vendor"] {
            if let Some(dir) = exe.parent() {
                v.push(dir.join(up).join("tinytex"));
            }
        }
    }
    v
}

fn archive_name(tag: &str) -> String {
    format!("TinyTeX-1-{tag}-{VERSION}.tar.xz")
}

/// Path of the archive for this platform, if vendored.
pub fn archive() -> Option<PathBuf> {
    let (tag, _) = platform()?;
    vendor_dirs().into_iter().map(|d| d.join(archive_name(tag))).find(|p| p.is_file())
}

fn ready(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".cmx-version")).is_ok_and(|v| v.trim() == VERSION)
}

/// Unpack `archive` into `dest` (atomically: into a temp dir, then rename). Safe if several cmx
/// processes race: the loser discards its copy.
fn unpack(archive: &Path, dest: &Path) -> Result<(), String> {
    let parent = dest.parent().ok_or("bad destination")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let tmp = parent.join(format!(".unpack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let tar =
        ["/usr/bin/tar", "/bin/tar"].iter().map(PathBuf::from).find(|p| p.is_file()).unwrap_or_else(|| "tar".into());
    let status = Command::new(&tar)
        .arg("-xJf")
        .arg(archive)
        .arg("-C")
        .arg(&tmp)
        .status()
        .map_err(|e| format!("running tar: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("tar failed to unpack {}", archive.display()));
    }
    let _ = std::fs::write(tmp.join("TinyTeX/.cmx-version"), VERSION);
    if ready(dest) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dest);
    match std::fs::rename(tmp.join("TinyTeX"), dest) {
        Ok(()) => {}
        Err(_) if ready(dest) => {}
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("moving TinyTeX into {}: {e}", dest.display()));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// The bundled engine binary (`pdflatex`, `xelatex`, `lualatex`), unpacking on first use.
/// `Ok(None)` when no archive is vendored for this platform.
pub fn engine(name: &str) -> Result<Option<PathBuf>, String> {
    let Some((tag, bin)) = platform() else { return Ok(None) };
    let Some(arch) = archive() else { return Ok(None) };
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    let candidates =
        [arch.parent().map(|p| p.join(tag)), Some(home.join(".cmx/tinytex").join(format!("{tag}-{VERSION}")))];
    for root in candidates.iter().flatten() {
        if ready(root) {
            return Ok(Some(root.join("bin").join(bin).join(name)));
        }
    }
    eprintln!("cmx: unpacking the bundled TinyTeX ({VERSION}, one time, ~250 MB on disk) ...");
    let mut last_err = String::new();
    for root in candidates.iter().flatten() {
        match unpack(&arch, root) {
            Ok(()) => return Ok(Some(root.join("bin").join(bin).join(name))),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

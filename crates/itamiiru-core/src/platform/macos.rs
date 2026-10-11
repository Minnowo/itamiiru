// FFI calls into libc, each with a SAFETY comment.
#![allow(unsafe_code)]

use std::path::PathBuf;

/// The itamiiru config folder: `~/Library/Application Support/itamiiru`.
pub fn config_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let dir = home
        .join("Library")
        .join("Application Support")
        .join("itamiiru");
    return Some(dir);
}

/// Dotfiles are hidden. TODO(Phase 8): also the UF_HIDDEN flag in st_flags.
pub fn is_hidden(name: &std::ffi::OsStr, _meta: &std::fs::Metadata) -> bool {
    return name.as_encoded_bytes().first() == Some(&b'.');
}

/// The home folder, from `$HOME`.
pub fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    return Some(PathBuf::from(home));
}

/// The local time zone's offset from UTC, in seconds, at the given Unix time.
/// The first call reads the time zone file, so call it once at startup.
pub fn utc_offset_seconds(unix_secs: i64) -> i64 {
    // time_t is 64-bit on every target we build for, so this cast keeps the value.
    let time = unix_secs as libc::time_t;
    // SAFETY: libc::tm is a plain C struct of integers and a pointer;
    // all zeroes is a valid value for it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r reads `time` and writes only into `tm`. Both are
    // valid local variables for the whole call. It's the thread-safe variant.
    let result = unsafe { libc::localtime_r(&time, &mut tm) };
    if result.is_null() {
        return 0;
    }
    return tm.tm_gmtoff as i64;
}

/// A regular file with any execute bit set.
pub fn is_executable(_name: &std::ffi::OsStr, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    return meta.is_file() && meta.permissions().mode() & 0o111 != 0;
}

/// Known system fonts covering CJK and other scripts egui lacks.
/// The files may not exist on every macOS version.
pub fn fallback_font_files() -> Vec<(PathBuf, u32)> {
    let files = [
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ];
    let mut fonts = Vec::new();
    for file in files {
        fonts.push((PathBuf::from(file), 0));
    }
    return fonts;
}

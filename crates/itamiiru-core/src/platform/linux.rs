// FFI calls into libc, each with a SAFETY comment.
#![allow(unsafe_code)]

use std::path::PathBuf;

/// The itamiiru config folder: `$XDG_CONFIG_HOME/itamiiru`, or `~/.config/itamiiru`.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let dir = PathBuf::from(dir);
        // The XDG spec says relative paths are invalid and must be ignored.
        if dir.is_absolute() {
            return Some(dir.join("itamiiru"));
        }
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    return Some(home.join(".config").join("itamiiru"));
}

/// Dotfiles are hidden.
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

/// Font files that cover scripts egui's built-in fonts lack (CJK, Thai,
/// Hebrew, Arabic, Devanagari), as (file, face index in a collection).
/// Asks fontconfig, so it runs `fc-match` once per script: call it on a
/// worker thread. Duplicates are removed; files may still not exist.
pub fn fallback_font_files() -> Vec<(PathBuf, u32)> {
    use std::os::unix::ffi::OsStrExt;
    let scripts = [
        ":lang=ja",
        ":lang=zh-cn",
        ":lang=zh-tw",
        ":lang=ko",
        ":lang=th",
        ":lang=he",
        ":lang=ar",
        ":lang=hi",
    ];
    let mut fonts: Vec<(PathBuf, u32)> = Vec::new();
    for script in scripts {
        let output = std::process::Command::new("fc-match")
            .args(["-f", "%{file}|%{index}", script])
            .output();
        let output = match output {
            Ok(output) if output.status.success() => output,
            _ => continue,
        };
        // "path|index"; the path is raw bytes, the index after the last '|'.
        let text = output.stdout;
        let Some(bar) = text.iter().rposition(|&b| b == b'|') else {
            continue;
        };
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(&text[..bar]));
        let index_text = String::from_utf8_lossy(&text[bar + 1..]);
        let index: u32 = index_text.trim().parse().unwrap_or(0);
        if path.as_os_str().is_empty() || fonts.contains(&(path.clone(), index)) {
            continue;
        }
        fonts.push((path, index));
    }
    return fonts;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_fonts_are_unique() {
        // Depends on the machine's fonts; only checks the parsing is sane.
        let fonts = fallback_font_files();
        for (i, font) in fonts.iter().enumerate() {
            assert!(!font.0.as_os_str().is_empty());
            assert!(!fonts[i + 1..].contains(font), "duplicate {font:?}");
        }
    }

    #[test]
    fn dotfiles_are_hidden() {
        let meta = std::fs::metadata("/").unwrap();
        assert!(is_hidden(std::ffi::OsStr::new(".x"), &meta));
        assert!(!is_hidden(std::ffi::OsStr::new("x.y"), &meta));
        assert!(!is_hidden(std::ffi::OsStr::new(""), &meta));
    }
}

//! Windows-style names for copies and duplicates.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// `report.txt` with n = 2 becomes `report (2).txt`. Folders (and names
/// with `keep_extension` false) get the number at the very end.
pub fn numbered(name: &OsStr, n: u32, keep_extension: bool) -> OsString {
    return with_suffix(name, &format!(" ({n})"), keep_extension);
}

/// The name for a copy pasted into the same folder: `report - Copy.txt`,
/// then `report - Copy (2).txt` and so on (n = 1 is the first copy).
pub fn copy_name(name: &OsStr, n: u32, keep_extension: bool) -> OsString {
    if n <= 1 {
        return with_suffix(name, " - Copy", keep_extension);
    }
    return with_suffix(name, &format!(" - Copy ({n})"), keep_extension);
}

fn with_suffix(name: &OsStr, suffix: &str, keep_extension: bool) -> OsString {
    let path = Path::new(name);
    let extension = if keep_extension {
        path.extension()
    } else {
        None
    };
    let Some(extension) = extension else {
        let mut out = name.to_os_string();
        out.push(suffix);
        return out;
    };
    // file_stem is Some whenever extension is.
    let stem = path.file_stem().unwrap_or(name);
    let mut out = stem.to_os_string();
    out.push(suffix);
    out.push(".");
    out.push(extension);
    return out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: OsString) -> String {
        return name.to_string_lossy().into_owned();
    }

    #[test]
    fn numbered_names() {
        assert_eq!(
            s(numbered(OsStr::new("report.txt"), 2, true)),
            "report (2).txt"
        );
        assert_eq!(
            s(numbered(OsStr::new("archive.tar.gz"), 3, true)),
            "archive.tar (3).gz"
        );
        assert_eq!(s(numbered(OsStr::new("Makefile"), 2, true)), "Makefile (2)");
        assert_eq!(s(numbered(OsStr::new(".bashrc"), 2, true)), ".bashrc (2)");
        assert_eq!(
            s(numbered(OsStr::new("photos.d"), 2, false)),
            "photos.d (2)"
        );
    }

    #[test]
    fn copy_names() {
        assert_eq!(s(copy_name(OsStr::new("a.txt"), 1, true)), "a - Copy.txt");
        assert_eq!(
            s(copy_name(OsStr::new("a.txt"), 2, true)),
            "a - Copy (2).txt"
        );
        assert_eq!(
            s(copy_name(OsStr::new("folder"), 1, false)),
            "folder - Copy"
        );
    }
}

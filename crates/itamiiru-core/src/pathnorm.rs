//! Tidying paths by their text, like a shell's `cd`: `.` goes away and
//! `..` removes the folder before it, so `/home/me/Downloads/..` becomes
//! `/home/me`. No disk access: symlinks aren't resolved, and the result is
//! the path as typed, just without the dots.

use std::path::{Component, Path, PathBuf};

/// `path` without `.` and `..` parts. `..` at the top of an absolute path
/// stays at the top (`/..` is `/`); in a relative path with nothing left to
/// remove it's kept (`../a` stays `../a`).
pub fn normalize(path: &Path) -> PathBuf {
    let mut parts: Vec<Component> = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                // Above the root is the root.
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                // Relative, and nothing to remove.
                _ => parts.push(part),
            },
            other => parts.push(other),
        }
    }
    let mut out = PathBuf::new();
    for part in parts {
        out.push(part.as_os_str());
    }
    if out.as_os_str().is_empty() {
        // "a/.." or ".": the folder itself.
        out.push(".");
    }
    return out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(path: &str) -> String {
        return normalize(Path::new(path)).to_string_lossy().into_owned();
    }

    #[test]
    fn dots_go_away() {
        assert_eq!(norm("/home/me/Downloads/.."), "/home/me");
        assert_eq!(norm("/home/me/Downloads/."), "/home/me/Downloads");
        assert_eq!(norm("/home/me/./Downloads/../Music"), "/home/me/Music");
        assert_eq!(norm("/home/me/Downloads/../.."), "/home");
        assert_eq!(norm("/a//b/"), "/a/b");
    }

    #[test]
    fn the_top_stays_the_top() {
        assert_eq!(norm("/.."), "/");
        assert_eq!(norm("/../../etc"), "/etc");
        assert_eq!(norm("/"), "/");
    }

    #[test]
    fn relative_paths() {
        assert_eq!(norm("a/../b"), "b");
        assert_eq!(norm("../a"), "../a");
        assert_eq!(norm("a/.."), ".");
        assert_eq!(norm("."), ".");
    }

    #[cfg(unix)]
    #[test]
    fn names_that_arent_utf8_survive() {
        use std::os::unix::ffi::OsStrExt;
        let raw = std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9/./x/..");
        let out = normalize(Path::new(raw));
        assert_eq!(out.as_os_str().as_bytes(), b"/tmp/caf\xe9");
    }
}

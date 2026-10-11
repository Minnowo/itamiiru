// FFI calls into libc, each with a SAFETY comment.
#![allow(unsafe_code)]

use std::path::PathBuf;

mod places;
pub use places::{bookmarks, disk_usage, drives, network_places, quick_access};

/// `$XDG_CONFIG_HOME`, or `~/.config`.
pub fn config_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let dir = PathBuf::from(dir);
        // The XDG spec says relative paths are invalid and must be ignored.
        if dir.is_absolute() {
            return Some(dir);
        }
    }
    return Some(home_dir()?.join(".config"));
}

/// The itamiiru config folder: `$XDG_CONFIG_HOME/itamiiru`, or `~/.config/itamiiru`.
pub fn config_dir() -> Option<PathBuf> {
    return Some(config_home()?.join("itamiiru"));
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

/// Raw bytes (e.g. a decoded file URI) to a path. Unix paths are bytes.
pub fn path_from_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    return Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
}

/// The real user id of this process.
pub fn current_uid() -> u32 {
    // SAFETY: getuid takes no arguments, has no preconditions and can't fail.
    return unsafe { libc::getuid() };
}

/// Opens a file in its default app with `xdg-open`. Returns once it started;
/// a thread waits for it to exit so it doesn't linger as a zombie process.
pub fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path);
    return spawn_and_reap(command);
}

fn spawn_and_reap(mut command: std::process::Command) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    return Ok(());
}

/// Recreates the symlink `src` at `dst`, pointing at the same target.
pub fn copy_symlink(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let target = std::fs::read_link(src)?;
    return std::os::unix::fs::symlink(target, dst);
}

/// Moves `path` to the trash, following the freedesktop trash spec
/// (https://specifications.freedesktop.org/trash-spec/latest/).
pub fn trash(path: &std::path::Path) -> std::io::Result<()> {
    let Some(home_trash) = home_trash_dir() else {
        return Err(std::io::Error::other(
            "can't find the home folder for the trash",
        ));
    };
    return trash_into(path, &home_trash);
}

/// `$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`.
fn home_trash_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return Some(dir.join("Trash"));
        }
    }
    return Some(home_dir()?.join(".local").join("share").join("Trash"));
}

/// Trashes `path` into `home_trash` when both are on the same filesystem,
/// otherwise into a trash folder at the top of the path's filesystem.
/// Split out from `trash` so tests can use a temporary home trash.
pub fn trash_into(path: &std::path::Path, home_trash: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::other("can't trash this path"));
    };
    // Resolve the parent (not the item itself: a symlink is trashed as a link).
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => std::fs::canonicalize(parent)?,
        _ => std::env::current_dir()?,
    };
    let real_path = parent.join(name);
    let device = std::fs::symlink_metadata(&real_path)?.dev();

    if device == nearest_existing_device(home_trash)? {
        // The home trash stores absolute paths.
        let info_path = real_path.as_os_str().as_encoded_bytes().to_vec();
        return move_to_trash(&real_path, home_trash, &info_path);
    }

    // Another drive: use a trash at the top of that drive, storing paths
    // relative to it. First $topdir/.Trash/$uid, if an admin made .Trash
    // (it must be a real directory with the sticky bit), else $topdir/.Trash-$uid.
    let topdir = mount_top(&parent, device);
    let relative = real_path.strip_prefix(&topdir).unwrap_or(&real_path);
    let info_path = relative.as_os_str().as_encoded_bytes().to_vec();
    let uid = current_uid();
    let shared = topdir.join(".Trash");
    if let Ok(meta) = std::fs::symlink_metadata(&shared) {
        use std::os::unix::fs::PermissionsExt;
        let sticky = meta.permissions().mode() & 0o1000 != 0;
        if meta.is_dir() && sticky {
            let dir = shared.join(uid.to_string());
            if create_private_dir(&dir).is_ok() {
                return move_to_trash(&real_path, &dir, &info_path);
            }
        }
    }
    let dir = topdir.join(format!(".Trash-{uid}"));
    create_private_dir(&dir)?;
    return move_to_trash(&real_path, &dir, &info_path);
}

/// The filesystem device of `path`, or of its nearest existing parent.
fn nearest_existing_device(path: &std::path::Path) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let mut current = Some(path);
    while let Some(p) = current {
        if let Ok(meta) = std::fs::metadata(p) {
            return Ok(meta.dev());
        }
        current = p.parent();
    }
    return Err(std::io::Error::other("no existing parent folder"));
}

/// The top folder of the filesystem `dir` is on: go up while the parent
/// is still on the same device.
fn mount_top(dir: &std::path::Path, device: u64) -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let mut top = dir.to_path_buf();
    while let Some(parent) = top.parent() {
        match std::fs::metadata(parent) {
            Ok(meta) if meta.dev() == device => top = parent.to_path_buf(),
            _ => break,
        }
    }
    return top;
}

/// Creates a folder (and missing parents) readable only by this user.
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    return std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir);
}

/// Moves `path` into `trash_dir/files` and writes its `.trashinfo`.
/// `info_path` is the original path as stored in the info file.
fn move_to_trash(
    path: &std::path::Path,
    trash_dir: &std::path::Path,
    info_path: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    let files = trash_dir.join("files");
    let info = trash_dir.join("info");
    create_private_dir(&files)?;
    create_private_dir(&info)?;
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::other("can't trash this path"));
    };
    let keep_extension = !std::fs::symlink_metadata(path)?.is_dir();
    let deleted_at = crate::format::format_local_iso(std::time::SystemTime::now());

    for n in 1..=10_000 {
        let candidate = if n == 1 {
            name.to_os_string()
        } else {
            crate::names::numbered(name, n, keep_extension)
        };
        let mut info_name = candidate.clone();
        info_name.push(".trashinfo");
        let info_file = info.join(&info_name);
        // Creating the info file with create_new reserves the name; the spec
        // relies on this being atomic.
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_file)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        let target = files.join(&candidate);
        if std::fs::symlink_metadata(&target).is_ok() {
            // A leftover without an info file; pick another name.
            drop(file);
            let _ = std::fs::remove_file(&info_file);
            continue;
        }
        let contents = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            crate::percent::encode(info_path),
            deleted_at
        );
        if let Err(err) = file.write_all(contents.as_bytes()) {
            drop(file);
            let _ = std::fs::remove_file(&info_file);
            return Err(err);
        }
        drop(file);
        if let Err(err) = std::fs::rename(path, &target) {
            let _ = std::fs::remove_file(&info_file);
            return Err(err);
        }
        return Ok(());
    }
    return Err(std::io::Error::other(
        "too many items with this name in the trash",
    ));
}

/// Virtual filesystems a search shouldn't walk into.
pub fn is_virtual_dir(path: &std::path::Path) -> bool {
    for dir in ["/proc", "/sys", "/dev", "/run"] {
        if path == std::path::Path::new(dir) {
            return true;
        }
    }
    return false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("itamiiru-trash-{}-{}", std::process::id(), label));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        return dir;
    }

    #[test]
    fn trash_file_writes_info() {
        let dir = test_dir("file");
        let trash = dir.join("Trash");
        let file = dir.join("a b.txt");
        std::fs::write(&file, b"hello").unwrap();

        trash_into(&file, &trash).unwrap();
        assert!(!file.exists());
        assert_eq!(
            std::fs::read(trash.join("files/a b.txt")).unwrap(),
            b"hello"
        );
        let info = std::fs::read_to_string(trash.join("info/a b.txt.trashinfo")).unwrap();
        let real = std::fs::canonicalize(&dir).unwrap().join("a b.txt");
        let expected_path = crate::percent::encode(real.as_os_str().as_encoded_bytes());
        assert!(info.starts_with("[Trash Info]\nPath="), "{info}");
        assert!(info.contains(&format!("Path={expected_path}\n")), "{info}");
        assert!(info.contains("%20"), "spaces are encoded: {info}");
        assert!(info.contains("DeletionDate="), "{info}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_same_name_twice_and_folders() {
        let dir = test_dir("twice");
        let trash = dir.join("Trash");
        for _ in 0..2 {
            std::fs::write(dir.join("x.txt"), b"").unwrap();
            trash_into(&dir.join("x.txt"), &trash).unwrap();
        }
        assert!(trash.join("files/x.txt").exists());
        assert!(trash.join("files/x (2).txt").exists());
        assert!(trash.join("info/x (2).txt.trashinfo").exists());

        std::fs::create_dir_all(dir.join("folder/inner")).unwrap();
        trash_into(&dir.join("folder"), &trash).unwrap();
        assert!(trash.join("files/folder/inner").is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_symlink_trashes_the_link() {
        let dir = test_dir("link");
        let trash = dir.join("Trash");
        std::fs::write(dir.join("target"), b"keep me").unwrap();
        std::os::unix::fs::symlink(dir.join("target"), dir.join("link")).unwrap();
        trash_into(&dir.join("link"), &trash).unwrap();
        assert!(dir.join("target").exists(), "the target stays");
        let meta = std::fs::symlink_metadata(trash.join("files/link")).unwrap();
        assert!(meta.file_type().is_symlink());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_missing_file_fails() {
        let dir = test_dir("missing");
        assert!(trash_into(&dir.join("nope"), &dir.join("Trash")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_bytes() {
        let path = path_from_bytes(b"/tmp/a\xffb".to_vec()).unwrap();
        assert_eq!(path.as_os_str().as_encoded_bytes(), b"/tmp/a\xffb");
    }

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

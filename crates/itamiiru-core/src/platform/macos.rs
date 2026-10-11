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

/// Raw bytes (e.g. a decoded file URI) to a path. Unix paths are bytes.
pub fn path_from_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    return Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
}

/// Opens a file in its default app with `open`.
pub fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = std::process::Command::new("open")
        .arg(path)
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

/// TODO(Phase 8, P8.3): decide between osascript and objc FFI.
pub fn trash(_path: &std::path::Path) -> std::io::Result<()> {
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "moving to the Trash isn't supported on macOS yet",
    ));
}

/// Home and the usual folders inside it; the places scan drops missing ones.
pub fn quick_access() -> Vec<crate::places::Place> {
    use crate::places::{Place, PlaceKind};
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let mut places = vec![Place {
        name: "Home".to_string(),
        path: home.clone(),
        kind: PlaceKind::Home,
    }];
    for name in [
        "Desktop",
        "Documents",
        "Downloads",
        "Music",
        "Pictures",
        "Videos",
    ] {
        places.push(Place {
            name: name.to_string(),
            path: home.join(name),
            kind: PlaceKind::Folder,
        });
    }
    return places;
}

/// No bookmark source yet.
pub fn bookmarks() -> Vec<crate::places::Place> {
    return Vec::new();
}

/// No network place source yet.
pub fn network_places() -> Vec<crate::places::Place> {
    return Vec::new();
}

/// The startup disk, plus what's in /Volumes. TODO(Phase 8): labels, kinds.
pub fn drives() -> Vec<crate::places::Drive> {
    use crate::places::{Drive, DriveKind};
    let mut drives = vec![Drive {
        name: "System".to_string(),
        mount_point: PathBuf::from("/"),
        fs_type: String::new(),
        kind: DriveKind::System,
        hidden: false,
    }];
    let Ok(items) = std::fs::read_dir("/Volumes") else {
        return drives;
    };
    for item in items.flatten() {
        // "Macintosh HD" is a symlink to /; skip links.
        let Ok(kind) = item.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        drives.push(Drive {
            name: item.file_name().to_string_lossy().into_owned(),
            mount_point: item.path(),
            fs_type: String::new(),
            kind: DriveKind::Removable,
            hidden: false,
        });
    }
    return drives;
}

/// Total and free space via statvfs.
pub fn disk_usage(path: &std::path::Path) -> Option<crate::places::DiskUsage> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: libc::statvfs is a plain C struct of integers; all zeroes is valid.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c_path is a valid NUL-terminated string and `stats` a valid
    // struct to write into; both live for the whole call.
    let result = unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) };
    if result != 0 {
        return None;
    }
    let block = stats.f_frsize as u64;
    return Some(crate::places::DiskUsage {
        total: (stats.f_blocks as u64).saturating_mul(block),
        free: (stats.f_bavail as u64).saturating_mul(block),
    });
}

/// Folders a search shouldn't walk into: devices, and /System/Volumes,
/// which mirrors the startup disk.
pub fn is_virtual_dir(path: &std::path::Path) -> bool {
    return path == std::path::Path::new("/dev") || path == std::path::Path::new("/System/Volumes");
}

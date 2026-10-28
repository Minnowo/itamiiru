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

/// Dotfiles are hidden. TODO: also the UF_HIDDEN flag in st_flags.
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

/// macOS's UI font (San Francisco) is a variable font egui can't use
/// well, so egui's own fonts stay the main ones there.
pub fn main_font_files() -> super::MainFonts {
    return super::MainFonts {
        sans: None,
        mono: None,
    };
}

/// For `itami --font-info`: there's no fontconfig to ask here.
pub fn font_report() -> Vec<String> {
    return vec!["(fonts come from fixed paths on this system)".to_string()];
}

/// Known system fonts covering CJK and other scripts egui lacks, each
/// as its own entry (name, [file]).
/// The files may not exist on every macOS version.
pub fn fallback_font_files() -> Vec<(String, Vec<super::FontFile>)> {
    let files = [
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ];
    let mut fonts = Vec::new();
    for file in files {
        fonts.push((file.to_string(), vec![(PathBuf::from(file), 0)]));
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

/// TODO: decide between osascript and objc FFI.
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

pub fn add_bookmark(_path: &std::path::Path, _label: Option<&str>) -> std::io::Result<()> {
    return Ok(());
}

pub fn remove_bookmark(_path: &std::path::Path) -> std::io::Result<()> {
    return Ok(());
}

pub fn is_bookmarked(_path: &std::path::Path) -> bool {
    return false;
}

/// No network place source yet.
pub fn network_places() -> Vec<crate::places::Place> {
    return Vec::new();
}

/// The startup disk, plus what's in /Volumes. TODO: labels, kinds.
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

/// No shared-mime-info here; the built-in table is used.
pub fn mime_glob_files() -> Vec<PathBuf> {
    return Vec::new();
}

/// Whether this user may create files in `dir` (access with W_OK).
pub fn is_writable_dir(dir: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: c_path is a valid NUL-terminated string that outlives the call;
    // access only reads it.
    let result = unsafe { libc::access(c_path.as_ptr(), libc::W_OK) };
    return result == 0;
}

/// Thumbnails, in the freedesktop layout: `~/Library/Caches/itamiiru/thumbnails`.
pub fn thumbnail_dir() -> Option<PathBuf> {
    let dir = home_dir()?
        .join("Library")
        .join("Caches")
        .join("itamiiru")
        .join("thumbnails");
    return Some(dir);
}

/// Creates a new file only this user can read (0600). Fails if it exists.
pub fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    return std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path);
}

/// Creates a folder and any missing parents, only this user can open (0700).
pub fn create_private_dir_all(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    return std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path);
}

/// Removable drives that aren't mounted yet: not done on this OS (Linux
/// uses UDisks2).
pub fn removable_volumes() -> Vec<crate::places::Volume> {
    return Vec::new();
}

pub fn mount_volume(_volume: &crate::places::Volume) -> Result<std::path::PathBuf, String> {
    return Err("mounting drives isn't supported on this system yet".to_string());
}

/// Where the pointer is on the screen, in pixels. Not done here yet:
/// files dropped from other apps go into the active pane's folder.
pub fn pointer_position() -> Option<(f32, f32)> {
    return None;
}

/// Dragging files out to other apps isn't done on this system yet.
pub fn start_drag_out(_paths: Vec<std::path::PathBuf>) -> Result<super::DragOut, String> {
    return Err("dragging out isn't supported on this system yet".to_string());
}

pub fn unmount_volume(_volume: &crate::places::Volume) -> Result<(), String> {
    return Err("unmounting drives isn't supported on this system yet".to_string());
}

pub fn eject_volume(_volume: &crate::places::Volume) -> Result<(), String> {
    return Err("ejecting drives isn't supported on this system yet".to_string());
}
/// Starts `program` with `args` in the background, cut off from the
/// terminal: no stdin, stdout or stderr, and its own process group, so
/// the terminal's hang-up signal (closing the window) doesn't reach it.
/// Doesn't wait for it; when this process exits, init adopts and reaps it.
pub fn spawn_detached(
    program: &std::path::Path,
    args: &[std::ffi::OsString],
) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    return Ok(());
}

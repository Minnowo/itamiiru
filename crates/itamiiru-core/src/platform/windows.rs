use std::path::PathBuf;

/// The itamiiru config folder: `%APPDATA%\itamiiru`.
pub fn config_dir() -> Option<PathBuf> {
    let appdata = PathBuf::from(std::env::var_os("APPDATA")?);
    return Some(appdata.join("itamiiru"));
}

/// Files with the hidden attribute are hidden.
pub fn is_hidden(_name: &std::ffi::OsStr, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    return meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0;
}

/// The home folder, from `%USERPROFILE%`.
pub fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    if home.is_empty() {
        return None;
    }
    return Some(PathBuf::from(home));
}

/// TODO(Phase 8): GetTimeZoneInformation. Until then Windows shows UTC.
pub fn utc_offset_seconds(_unix_secs: i64) -> i64 {
    return 0;
}

/// Windows has no execute bit; go by extension.
pub fn is_executable(name: &std::ffi::OsStr, meta: &std::fs::Metadata) -> bool {
    if !meta.is_file() {
        return false;
    }
    let Some(ext) = std::path::Path::new(name).extension() else {
        return false;
    };
    let ext = ext.to_string_lossy().to_ascii_lowercase();
    return ["exe", "com", "bat", "cmd", "msi", "ps1"].contains(&ext.as_str());
}

/// Known system fonts covering CJK and other scripts egui lacks.
pub fn fallback_font_files() -> Vec<(PathBuf, u32)> {
    let windir = match std::env::var_os("WINDIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from("C:\\Windows"),
    };
    let fonts_dir = windir.join("Fonts");
    // Japanese, Simplified and Traditional Chinese, Korean, Indic, Thai.
    let files = [
        "YuGothM.ttc",
        "msyh.ttc",
        "msjh.ttc",
        "malgun.ttf",
        "Nirmala.ttf",
        "LeelawUI.ttf",
    ];
    let mut fonts = Vec::new();
    for file in files {
        fonts.push((fonts_dir.join(file), 0));
    }
    return fonts;
}

/// Raw bytes (e.g. a decoded file URI) to a path. Windows paths here must be UTF-8.
pub fn path_from_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    let text = String::from_utf8(bytes).ok()?;
    return Some(PathBuf::from(text));
}

/// Opens a file in its default app. TODO(Phase 8): ShellExecuteW.
pub fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = std::process::Command::new("explorer")
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
    // Windows has separate file and folder symlinks.
    if std::fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false) {
        return std::os::windows::fs::symlink_dir(target, dst);
    }
    return std::os::windows::fs::symlink_file(target, dst);
}

/// TODO(Phase 8, P8.1): SHFileOperationW with FOF_ALLOWUNDO.
pub fn trash(_path: &std::path::Path) -> std::io::Result<()> {
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "moving to the Recycle Bin isn't supported on Windows yet",
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

/// TODO(Phase 8, P8.1): GetLogicalDrives and friends.
pub fn drives() -> Vec<crate::places::Drive> {
    return Vec::new();
}

/// TODO(Phase 8, P8.1): GetDiskFreeSpaceExW.
pub fn disk_usage(_path: &std::path::Path) -> Option<crate::places::DiskUsage> {
    return None;
}

/// Nothing to skip on Windows.
pub fn is_virtual_dir(_path: &std::path::Path) -> bool {
    return false;
}

/// No shared-mime-info here; the built-in table is used.
pub fn mime_glob_files() -> Vec<PathBuf> {
    return Vec::new();
}

/// Whether `dir` exists and isn't read-only. TODO(Phase 8): check ACLs.
pub fn is_writable_dir(dir: &std::path::Path) -> bool {
    return match std::fs::metadata(dir) {
        Ok(meta) => meta.is_dir() && !meta.permissions().readonly(),
        Err(_) => false,
    };
}

/// Thumbnails, in the freedesktop layout: `%LOCALAPPDATA%\itamiiru\thumbnails`.
pub fn thumbnail_dir() -> Option<PathBuf> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    return Some(local.join("itamiiru").join("thumbnails"));
}

/// Creates a new file. Fails if it exists. TODO(Phase 8): an ACL for this
/// user only; the default under %LOCALAPPDATA% is already private.
pub fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    return std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);
}

/// Creates a folder and any missing parents.
pub fn create_private_dir_all(path: &std::path::Path) -> std::io::Result<()> {
    return std::fs::create_dir_all(path);
}

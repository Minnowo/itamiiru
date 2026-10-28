//! Linux places: user-dirs, GTK bookmarks, /proc/self/mountinfo, gvfs.
//! The parsers take text, so tests can feed them fixtures.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::places::{DiskUsage, Drive, DriveKind, Place, PlaceKind};

/// Files bigger than this are not read (they're all a few KiB normally).
const MAX_FILE_SIZE: u64 = 1024 * 1024;

/// Filesystems that are never shown as drives.
const PSEUDO_FILESYSTEMS: &[&str] = &[
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "overlay",
    "squashfs",
    "autofs",
    "bpf",
    "tracefs",
    "debugfs",
    "securityfs",
    "pstore",
    "fusectl",
    "configfs",
    "mqueue",
    "hugetlbfs",
    "binfmt_misc",
    "nsfs",
    "ramfs",
    "efivarfs",
    "rpc_pipefs",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "fuse.snapfuse",
    "fuse.lxcfs",
];

/// Filesystems shown under Network, wherever they're mounted.
const NETWORK_FILESYSTEMS: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "fuse.sshfs",
    "fuse.rclone",
    "davfs",
    "9p",
    "afs",
    "ceph",
    "glusterfs",
];

/// Reads a small text file, or None if it's missing or too big.
fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    // /proc files report size 0, so only real sizes are checked.
    if meta.len() > MAX_FILE_SIZE {
        return None;
    }
    let mut text = String::new();
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    file.take(MAX_FILE_SIZE).read_to_string(&mut text).ok()?;
    return Some(text);
}

pub fn quick_access() -> Vec<Place> {
    let Some(home) = super::home_dir() else {
        return Vec::new();
    };
    let mut places = vec![Place {
        name: "Home".to_string(),
        path: home.clone(),
        kind: PlaceKind::Home,
    }];
    let text = match super::config_home() {
        Some(config) => read_small(&config.join("user-dirs.dirs")).unwrap_or_default(),
        None => String::new(),
    };
    let dirs = parse_user_dirs(&text, &home);
    // Explorer's order, with XDG's names.
    let wanted = [
        ("DESKTOP", "Desktop"),
        ("DOCUMENTS", "Documents"),
        ("DOWNLOAD", "Downloads"),
        ("MUSIC", "Music"),
        ("PICTURES", "Pictures"),
        ("VIDEOS", "Videos"),
    ];
    for (key, default_name) in wanted {
        let path = match dirs.get(key) {
            Some(path) => path.clone(),
            None => home.join(default_name),
        };
        // XDG sets unused folders to $HOME itself; Home is already listed.
        if path == home {
            continue;
        }
        let name = match path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => default_name.to_string(),
        };
        places.push(Place {
            name,
            path,
            kind: PlaceKind::Folder,
        });
    }
    return places;
}

/// Parses `user-dirs.dirs`: lines like `XDG_DOWNLOAD_DIR="$HOME/Downloads"`.
/// Returns "DOWNLOAD" -> path. Values are `$HOME/...` or absolute, with
/// shell backslash escapes.
pub fn parse_user_dirs(text: &str, home: &Path) -> HashMap<String, PathBuf> {
    let mut dirs = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(name) = key
            .trim()
            .strip_prefix("XDG_")
            .and_then(|k| k.strip_suffix("_DIR"))
        else {
            continue;
        };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        let value = unescape_shell(value);
        let path = if let Some(rest) = value.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            continue;
        };
        dirs.insert(name.to_string(), path);
    }
    return dirs;
}

/// Removes shell backslash escapes: `\"` -> `"`, `\\` -> `\`, `\$` -> `$`.
fn unescape_shell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        out.push(c);
    }
    return out;
}

pub fn bookmarks() -> Vec<Place> {
    let Some(config) = super::config_home() else {
        return Vec::new();
    };
    let text = read_small(&config.join("gtk-3.0").join("bookmarks")).unwrap_or_default();
    return parse_bookmarks(&text);
}

/// Adds `path` as a GTK bookmark with an optional label.
pub fn add_bookmark(path: &Path, label: Option<&str>) -> std::io::Result<()> {
    let Some(config) = super::config_home() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot find config directory",
        ));
    };
    let file_path = config.join("gtk-3.0").join("bookmarks");
    let text = read_small(&file_path).unwrap_or_default();
    let (new_text, changed) = add_bookmark_to_text(&text, path, label);
    if changed {
        return write_small_atomic(&file_path, new_text.as_bytes());
    }
    return Ok(());
}

/// Removes `path` from GTK bookmarks if present.
pub fn remove_bookmark(path: &Path) -> std::io::Result<()> {
    let Some(config) = super::config_home() else {
        return Ok(());
    };
    let file_path = config.join("gtk-3.0").join("bookmarks");
    let text = match read_small(&file_path) {
        Some(t) => t,
        None => return Ok(()),
    };
    let (new_text, changed) = remove_bookmark_from_text(&text, path);
    if changed {
        return write_small_atomic(&file_path, new_text.as_bytes());
    }
    return Ok(());
}

/// Checks whether `path` is in GTK bookmarks.
pub fn is_bookmarked(path: &Path) -> bool {
    let Some(config) = super::config_home() else {
        return false;
    };
    let file_path = config.join("gtk-3.0").join("bookmarks");
    let Some(text) = read_small(&file_path) else {
        return false;
    };
    let uri = crate::percent::path_to_file_uri(path);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (existing_uri, _) = match trimmed.split_once(' ') {
            Some((u, l)) => (u, l.trim()),
            None => (trimmed, ""),
        };
        if existing_uri == uri {
            return true;
        }
    }
    return false;
}

pub fn add_bookmark_to_text(text: &str, path: &Path, label: Option<&str>) -> (String, bool) {
    let uri = crate::percent::path_to_file_uri(path);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (existing_uri, _) = match trimmed.split_once(' ') {
            Some((u, l)) => (u, l.trim()),
            None => (trimmed, ""),
        };
        if existing_uri == uri {
            return (text.to_string(), false);
        }
    }
    let mut new_text = text.to_string();
    if !new_text.is_empty() && !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    new_text.push_str(&uri);
    if let Some(lbl) = label {
        let lbl = lbl.trim();
        if !lbl.is_empty() {
            new_text.push(' ');
            new_text.push_str(lbl);
        }
    }
    new_text.push('\n');
    return (new_text, true);
}

pub fn remove_bookmark_from_text(text: &str, path: &Path) -> (String, bool) {
    let uri = crate::percent::path_to_file_uri(path);
    let mut new_text = String::new();
    let mut changed = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (existing_uri, _) = match trimmed.split_once(' ') {
            Some((u, l)) => (u, l.trim()),
            None => (trimmed, ""),
        };
        if existing_uri == uri {
            changed = true;
            continue;
        }
        new_text.push_str(trimmed);
        new_text.push('\n');
    }
    return (new_text, changed);
}

fn write_small_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    let temp_path = PathBuf::from(temp);
    std::fs::write(&temp_path, content)?;
    return std::fs::rename(&temp_path, path);
}

/// Parses GTK bookmarks: `file:///path optional label` per line.
/// Non-file URIs (sftp://, smb://) are skipped; they need gvfs.
pub fn parse_bookmarks(text: &str) -> Vec<Place> {
    let mut places = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (uri, label) = match line.split_once(' ') {
            Some((uri, label)) => (uri, label.trim()),
            None => (line, ""),
        };
        let Some(path) = crate::percent::file_uri_to_path(uri) else {
            continue;
        };
        let name = if !label.is_empty() {
            label.to_string()
        } else {
            match path.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => path.to_string_lossy().into_owned(),
            }
        };
        places.push(Place {
            name,
            path,
            kind: PlaceKind::Bookmark,
        });
    }
    return places;
}

/// One line of /proc/self/mountinfo, the fields we use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// "major:minor" of the device.
    pub device: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub source: String,
}

/// Parses /proc/self/mountinfo. Format per line:
/// `id parent major:minor root mount_point options [optional...] - fstype source superoptions`
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut mounts = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(dash) = fields.iter().position(|f| *f == "-") else {
            continue;
        };
        if dash < 6 || fields.len() < dash + 3 {
            continue;
        }
        mounts.push(Mount {
            device: fields[2].to_string(),
            mount_point: unescape_octal(fields[4]),
            fs_type: fields[dash + 1].to_string(),
            source: String::from_utf8_lossy(
                unescape_octal(fields[dash + 2])
                    .as_os_str()
                    .as_encoded_bytes(),
            )
            .into_owned(),
        });
    }
    return mounts;
}

/// The kernel writes space, tab, newline and backslash in paths as
/// `\040`, `\011`, `\012`, `\134`. Any `\ooo` is decoded to that byte.
fn unescape_octal(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &bytes[i + 1..i + 4];
            let mut value: u32 = 0;
            let mut ok = true;
            for &d in digits {
                if !(b'0'..=b'7').contains(&d) {
                    ok = false;
                    break;
                }
                value = value * 8 + (d - b'0') as u32;
            }
            if ok && value <= 255 {
                out.push(value as u8);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    return PathBuf::from(std::ffi::OsString::from_vec(out));
}

/// Picks the mounts to show as drives, with names and kinds.
/// `labels` maps a device path (/dev/sdb1) to its filesystem label.
pub fn drives_from_mounts(
    mounts: &[Mount],
    labels: &HashMap<PathBuf, String>,
    user: &str,
) -> Vec<Drive> {
    let media_user = PathBuf::from("/run/media").join(user);
    let mut drives: Vec<Drive> = Vec::new();
    let mut seen_devices: HashSet<String> = HashSet::new();
    let mut seen_points: HashSet<PathBuf> = HashSet::new();

    for mount in mounts {
        if PSEUDO_FILESYSTEMS.contains(&mount.fs_type.as_str()) {
            continue;
        }
        let point = &mount.mount_point;
        let network = NETWORK_FILESYSTEMS.contains(&mount.fs_type.as_str());
        let (kind, hidden) = if network {
            (DriveKind::Network, false)
        } else if point == Path::new("/") || point == Path::new("/home") {
            (DriveKind::System, false)
        } else if point.starts_with("/boot") {
            (DriveKind::System, true)
        } else if point.starts_with(&media_user) || point.starts_with("/media") {
            (DriveKind::Removable, false)
        } else if point.starts_with("/mnt") {
            (DriveKind::System, false)
        } else {
            // /var, /srv, /snap/... and so on aren't shown.
            continue;
        };
        // The same filesystem mounted twice (bind mounts, btrfs subvolumes):
        // the first one wins, which is the shortest path for / and /home.
        if !network && !seen_devices.insert(mount.device.clone()) {
            continue;
        }
        if !seen_points.insert(point.clone()) {
            continue;
        }
        let label = labels.get(Path::new(&mount.source)).cloned();
        let name = drive_name(point, label);
        drives.push(Drive {
            name,
            mount_point: point.clone(),
            fs_type: mount.fs_type.clone(),
            kind,
            hidden,
        });
    }
    return drives;
}

fn drive_name(point: &Path, label: Option<String>) -> String {
    if let Some(label) = label {
        return label;
    }
    if point == Path::new("/") {
        return "System".to_string();
    }
    return match point.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => point.to_string_lossy().into_owned(),
    };
}

pub fn drives() -> Vec<Drive> {
    let text = read_small(Path::new("/proc/self/mountinfo")).unwrap_or_default();
    let mut mounts = parse_mountinfo(&text);
    // Compare real device paths: /dev/mapper/x and /dev/disk/... are symlinks.
    for mount in &mut mounts {
        if mount.source.starts_with('/')
            && let Ok(real) = std::fs::canonicalize(&mount.source)
        {
            mount.source = real.to_string_lossy().into_owned();
        }
    }
    let user = std::env::var("USER").unwrap_or_default();
    return drives_from_mounts(&mounts, &disk_labels(), &user);
}

/// /dev/disk/by-label/<label> -> /dev/sdX symlinks, as device -> label.
fn disk_labels() -> HashMap<PathBuf, String> {
    let mut labels = HashMap::new();
    let Ok(items) = std::fs::read_dir("/dev/disk/by-label") else {
        return labels;
    };
    for item in items.flatten() {
        let Ok(device) = std::fs::canonicalize(item.path()) else {
            continue;
        };
        let label = unescape_udev(&item.file_name().to_string_lossy());
        labels.insert(device, label);
    }
    return labels;
}

/// udev writes unsafe label characters as `\xHH` (e.g. `My\x20Disk`).
fn unescape_udev(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
            let hex = std::str::from_utf8(&bytes[i + 2..i + 4]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(hex, 16)
                && hex.bytes().all(|b| b.is_ascii_hexdigit())
            {
                out.push(value);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    return String::from_utf8_lossy(&out).into_owned();
}

/// gvfs mounts (shares opened in GNOME Files etc.) under /run/user/$UID/gvfs.
pub fn network_places() -> Vec<Place> {
    let gvfs = PathBuf::from(format!("/run/user/{}/gvfs", super::current_uid()));
    let mut places = Vec::new();
    let Ok(items) = std::fs::read_dir(&gvfs) else {
        return places;
    };
    for item in items.flatten() {
        let raw = item.file_name().to_string_lossy().into_owned();
        places.push(Place {
            name: gvfs_display_name(&raw),
            path: item.path(),
            kind: PlaceKind::Network,
        });
    }
    places.sort_by(|a, b| a.name.cmp(&b.name));
    return places;
}

/// `smb-share:server=nas,share=media` -> "media on nas (smb)";
/// `sftp:host=example.com,user=me` -> "example.com (sftp)".
pub fn gvfs_display_name(raw: &str) -> String {
    let Some((protocol, params)) = raw.split_once(':') else {
        return raw.to_string();
    };
    let mut values: HashMap<&str, &str> = HashMap::new();
    for pair in params.split(',') {
        if let Some((key, value)) = pair.split_once('=') {
            values.insert(key, value);
        }
    }
    let protocol = protocol.strip_suffix("-share").unwrap_or(protocol);
    let host = values.get("host").or(values.get("server"));
    let name = match (values.get("share"), host) {
        (Some(share), Some(host)) => format!("{share} on {host}"),
        (None, Some(host)) => host.to_string(),
        _ => return raw.to_string(),
    };
    return format!("{name} ({protocol})");
}

/// Total and free space via statvfs.
pub fn disk_usage(path: &Path) -> Option<DiskUsage> {
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
    return Some(DiskUsage {
        total: (stats.f_blocks as u64).saturating_mul(block),
        free: (stats.f_bavail as u64).saturating_mul(block),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs() {
        let text = r#"
# comment
XDG_DESKTOP_DIR="$HOME/Desktop"
XDG_DOWNLOAD_DIR="$HOME/My \"Stuff\""
XDG_MUSIC_DIR="/data/music"
XDG_VIDEOS_DIR="$HOME/"
XDG_BROKEN_DIR=nonsense
not a line
"#;
        let home = Path::new("/home/me");
        let dirs = parse_user_dirs(text, home);
        assert_eq!(dirs["DESKTOP"], Path::new("/home/me/Desktop"));
        assert_eq!(dirs["DOWNLOAD"], Path::new("/home/me/My \"Stuff\""));
        assert_eq!(dirs["MUSIC"], Path::new("/data/music"));
        assert_eq!(dirs["VIDEOS"], Path::new("/home/me"));
        assert!(!dirs.contains_key("BROKEN"));
    }

    #[test]
    fn gtk_bookmarks() {
        let text = "file:///home/me/work Work stuff\nfile:///tmp/a%20b\nsftp://host/x\n\n";
        let places = parse_bookmarks(text);
        assert_eq!(places.len(), 2);
        assert_eq!(places[0].name, "Work stuff");
        assert_eq!(places[0].path, Path::new("/home/me/work"));
        assert_eq!(places[1].name, "a b");
        assert_eq!(places[1].path, Path::new("/tmp/a b"));
    }

    #[test]
    fn adding_and_removing_bookmarks() {
        let text = "file:///home/me/work Work stuff\n";
        let (added, changed) = add_bookmark_to_text(text, Path::new("/tmp/test"), Some("Test"));
        assert!(changed);
        assert_eq!(
            added,
            "file:///home/me/work Work stuff\nfile:///tmp/test Test\n"
        );

        let (dup, changed_dup) = add_bookmark_to_text(&added, Path::new("/tmp/test"), None);
        assert!(!changed_dup);
        assert_eq!(dup, added);

        let (removed, changed_rem) = remove_bookmark_from_text(&added, Path::new("/home/me/work"));
        assert!(changed_rem);
        assert_eq!(removed, "file:///tmp/test Test\n");

        let (not_found, changed_nf) = remove_bookmark_from_text(&removed, Path::new("/missing"));
        assert!(!changed_nf);
        assert_eq!(not_found, removed);
    }

    const MOUNTINFO: &str = "\
22 1 0:22 / /sys rw,nosuid shared:7 - sysfs sysfs rw
26 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
30 26 259:6 / /home rw,relatime shared:8 - ext4 /dev/nvme0n1p6 rw
31 26 259:1 / /boot/efi rw,relatime shared:9 - vfat /dev/nvme0n1p1 rw
32 26 0:37 / /tmp rw shared:10 - tmpfs tmpfs rw
40 26 8:17 / /run/media/me/My\\040Stick rw shared:20 - vfat /dev/sdb1 rw
41 26 259:2 /srv/data /mnt/bind rw shared:1 - ext4 /dev/nvme0n1p2 rw
42 26 0:50 / /mnt/nas rw shared:30 - nfs4 nas:/export rw
43 26 0:51 / /run/user/1000/doc rw shared:31 - fuse.portal portal rw
44 26 259:9 / /var/lib/docker rw shared:32 - ext4 /dev/nvme0n1p9 rw
broken line
";

    #[test]
    fn mountinfo_parsing() {
        let mounts = parse_mountinfo(MOUNTINFO);
        assert_eq!(mounts.len(), 10);
        assert_eq!(mounts[1].mount_point, Path::new("/"));
        assert_eq!(mounts[1].fs_type, "ext4");
        assert_eq!(mounts[1].source, "/dev/nvme0n1p2");
        assert_eq!(mounts[5].mount_point, Path::new("/run/media/me/My Stick"));
    }

    #[test]
    fn choosing_drives() {
        let mounts = parse_mountinfo(MOUNTINFO);
        let mut labels = HashMap::new();
        labels.insert(PathBuf::from("/dev/sdb1"), "STICK".to_string());
        let drives = drives_from_mounts(&mounts, &labels, "me");
        let summary: Vec<(String, DriveKind, bool)> = drives
            .iter()
            .map(|d| (d.name.clone(), d.kind, d.hidden))
            .collect();
        assert_eq!(
            summary,
            [
                ("System".to_string(), DriveKind::System, false),
                ("home".to_string(), DriveKind::System, false),
                ("efi".to_string(), DriveKind::System, true),
                ("STICK".to_string(), DriveKind::Removable, false),
                ("nas".to_string(), DriveKind::Network, false),
            ]
        );
    }

    #[test]
    fn octal_and_udev_escapes() {
        assert_eq!(unescape_octal("a\\040b\\134c"), Path::new("a b\\c"));
        assert_eq!(unescape_octal("trailing\\04"), Path::new("trailing\\04"));
        assert_eq!(unescape_octal("bad\\999"), Path::new("bad\\999"));
        assert_eq!(unescape_udev("My\\x20Disk"), "My Disk");
        assert_eq!(unescape_udev("x\\xZZ"), "x\\xZZ");
    }

    #[test]
    fn gvfs_names() {
        assert_eq!(
            gvfs_display_name("smb-share:server=nas,share=media"),
            "media on nas (smb)"
        );
        assert_eq!(
            gvfs_display_name("sftp:host=example.com,user=me"),
            "example.com (sftp)"
        );
        assert_eq!(gvfs_display_name("weird"), "weird");
    }

    #[test]
    fn usage_of_root() {
        let usage = disk_usage(Path::new("/")).expect("statvfs on /");
        assert!(usage.total > 0);
        assert!(usage.free <= usage.total);
        assert!(disk_usage(Path::new("/definitely/not/here")).is_none());
    }
}

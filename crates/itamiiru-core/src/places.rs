//! The navigation pane's contents: quick access folders, bookmarks, drives
//! and network places. Found on worker threads; the
//! OS-specific parts live in `platform`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::listing::Wake;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Folder,
    Bookmark,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub kind: PlaceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveKind {
    System,
    Removable,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub name: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub kind: DriveKind,
    /// Only shown with hidden files on (e.g. /boot).
    pub hidden: bool,
}

/// A filesystem on a removable drive (a USB stick, an SD card), mounted
/// or not. Found and mounted through the OS's disk service (UDisks2 on
/// Linux), so plugged-in drives show up before they're mounted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// The disk service's name for it (a UDisks2 object path).
    pub id: String,
    /// The drive it's on; ejecting unmounts every volume on it. Empty for
    /// volumes without a drive, like loop devices.
    pub drive_id: String,
    /// The label, or "<size> Volume" without one.
    pub name: String,
    pub device: PathBuf,
    pub size: u64,
    pub fs_type: String,
    /// Where it's mounted, or None.
    pub mount_point: Option<PathBuf>,
    /// The drive can be powered off or its media ejected after unmounting.
    pub can_power_off: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskUsage {
    pub total: u64,
    /// Space this user can still write.
    pub free: u64,
}

impl DiskUsage {
    /// 0.0 (empty) to 1.0 (full).
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let used = self.total.saturating_sub(self.free);
        return (used as f64 / self.total as f64) as f32;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    pub quick: Vec<Place>,
    pub bookmarks: Vec<Place>,
    pub drives: Vec<Drive>,
    pub network: Vec<Place>,
    /// Filesystems on removable drives, mounted or not.
    pub volumes: Vec<Volume>,
}

impl Places {
    /// The removable volume mounted at `mount_point`, for its Eject button.
    pub fn volume_at(&self, mount_point: &Path) -> Option<&Volume> {
        for volume in &self.volumes {
            if volume.mount_point.as_deref() == Some(mount_point) {
                return Some(volume);
            }
        }
        return None;
    }

    /// Removable volumes that aren't mounted yet.
    pub fn unmounted(&self) -> Vec<&Volume> {
        let mut out = Vec::new();
        for volume in &self.volumes {
            if volume.mount_point.is_none() {
                out.push(volume);
            }
        }
        return out;
    }

    /// Whether `path` is currently bookmarked.
    pub fn is_bookmarked(&self, path: &Path) -> bool {
        for bookmark in &self.bookmarks {
            if bookmark.path == path {
                return true;
            }
        }
        return false;
    }
}

pub fn add_bookmark(path: &Path, label: Option<&str>) -> std::io::Result<()> {
    return crate::platform::add_bookmark(path, label);
}

pub fn remove_bookmark(path: &Path) -> std::io::Result<()> {
    return crate::platform::remove_bookmark(path);
}

pub fn is_bookmarked(path: &Path) -> bool {
    return crate::platform::is_bookmarked(path);
}

#[derive(Debug)]
pub enum PlacesMsg {
    Places(Places),
    Usage {
        mount_point: PathBuf,
        usage: Option<DiskUsage>,
    },
    /// A mount finished: where it's mounted, or why it failed.
    Mounted {
        volume: Volume,
        result: Result<PathBuf, String>,
    },
    /// An unmount or eject finished. Ok means it's safe to unplug; after
    /// an eject (`power_off`) the drive is gone from the list too.
    Ejected {
        volume: Volume,
        power_off: bool,
        result: Result<(), String>,
    },
}

/// Finds all places on a new thread and sends one `PlacesMsg::Places`.
/// Disk usage is asked for separately with `start_usage`.
/// Whether two paths are on the same drive, going by the mount points
/// found by the last scan (no filesystem access): each path belongs to the
/// longest mount point it's under. Paths under no known mount point count
/// as one drive.
pub fn same_drive(drives: &[Drive], a: &Path, b: &Path) -> bool {
    return mount_point_of(drives, a) == mount_point_of(drives, b);
}

fn mount_point_of<'a>(drives: &'a [Drive], path: &Path) -> Option<&'a Path> {
    let mut best: Option<&Path> = None;
    for drive in drives {
        let mount = drive.mount_point.as_path();
        if !path.starts_with(mount) {
            continue;
        }
        let longer = match best {
            Some(best) => mount.as_os_str().len() > best.as_os_str().len(),
            None => true,
        };
        if longer {
            best = Some(mount);
        }
    }
    return best;
}

pub fn start_scan(tx: Sender<PlacesMsg>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("places".to_string())
        .spawn(move || {
            let places = scan();
            if tx.send(PlacesMsg::Places(places)).is_ok() {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("places: could not start thread: {err}");
    }
}

fn scan() -> Places {
    crate::debug_slow_fs();
    let mut quick = Vec::new();
    for place in crate::platform::quick_access() {
        // Only folders that exist; user-dirs often names ones never created.
        if is_dir(&place.path) {
            quick.push(place);
        }
    }
    return Places {
        quick,
        bookmarks: crate::platform::bookmarks(),
        drives: crate::platform::drives(),
        network: crate::platform::network_places(),
        volumes: crate::platform::removable_volumes(),
    };
}

/// Mounts a volume on its own thread; the disk service may wait for a
/// password prompt. Sends `PlacesMsg::Mounted`.
pub fn start_mount(volume: Volume, tx: Sender<PlacesMsg>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("mount".to_string())
        .spawn(move || {
            let result = crate::platform::mount_volume(&volume);
            if tx.send(PlacesMsg::Mounted { volume, result }).is_ok() {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("places: could not start mount thread: {err}");
    }
}

/// On its own thread: with `power_off`, ejects (unmounts every volume on
/// the drive, then powers it off); without, unmounts just this volume and
/// leaves the drive plugged in and listed. Sends `PlacesMsg::Ejected`.
pub fn start_eject(volume: Volume, power_off: bool, tx: Sender<PlacesMsg>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("eject".to_string())
        .spawn(move || {
            let result = if power_off {
                crate::platform::eject_volume(&volume)
            } else {
                crate::platform::unmount_volume(&volume)
            };
            let msg = PlacesMsg::Ejected {
                volume,
                power_off,
                result,
            };
            if tx.send(msg).is_ok() {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("places: could not start eject thread: {err}");
    }
}

fn is_dir(path: &Path) -> bool {
    return match std::fs::metadata(path) {
        Ok(meta) => meta.is_dir(),
        Err(_) => false,
    };
}

/// Measures one drive on its own thread: statvfs on a dead network mount
/// can block forever, and must only block its own thread.
pub fn start_usage(mount_point: PathBuf, tx: Sender<PlacesMsg>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("disk-usage".to_string())
        .spawn(move || {
            crate::debug_slow_fs();
            let usage = crate::platform::disk_usage(&mount_point);
            if tx.send(PlacesMsg::Usage { mount_point, usage }).is_ok() {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("places: could not start usage thread: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_drive_by_mount_point() {
        let drive = |mount: &str| Drive {
            name: String::new(),
            mount_point: PathBuf::from(mount),
            fs_type: String::new(),
            kind: DriveKind::System,
            hidden: false,
        };
        let drives = [drive("/"), drive("/home"), drive("/media/usb")];
        let same = |a: &str, b: &str| same_drive(&drives, Path::new(a), Path::new(b));
        assert!(same("/home/me/a", "/home/me/b/c"));
        assert!(same("/etc", "/usr/lib"));
        assert!(!same("/home/me", "/etc"));
        assert!(!same("/home/me", "/media/usb/photos"));
        // Whole components only: /homework is on /, not /home.
        assert!(same("/homework", "/etc"));
        assert!(same("/a", "/b"));
        assert!(same_drive(&[], Path::new("/a"), Path::new("/b")));
    }

    #[test]
    fn used_fraction() {
        let usage = DiskUsage {
            total: 100,
            free: 25,
        };
        assert_eq!(usage.used_fraction(), 0.75);
        assert_eq!(DiskUsage { total: 0, free: 0 }.used_fraction(), 0.0);
        // free > total (odd filesystems) doesn't underflow.
        assert_eq!(
            DiskUsage {
                total: 10,
                free: 20
            }
            .used_fraction(),
            0.0
        );
    }

    #[test]
    fn scan_runs() {
        let (tx, rx) = std::sync::mpsc::channel();
        start_scan(tx, Box::new(|| {}));
        match rx.recv().expect("one message") {
            PlacesMsg::Places(places) => {
                for place in &places.quick {
                    assert!(place.path.is_dir(), "{place:?}");
                }
            }
            other => panic!("expected Places, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod show {
    /// Not a real test: `cargo test -p itamiiru-core -- --ignored --nocapture print_places`
    #[test]
    #[ignore]
    fn print_places() {
        let places = super::scan();
        println!("{places:#?}");
        for drive in &places.drives {
            println!(
                "{} -> {:?}",
                drive.name,
                crate::platform::disk_usage(&drive.mount_point)
            );
        }
    }
}

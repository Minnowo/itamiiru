//! The navigation pane's contents: quick access folders, bookmarks, drives
//! and network places (PLAN.md section 6). Found on worker threads; the
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
}

#[derive(Debug)]
pub enum PlacesMsg {
    Places(Places),
    Usage {
        mount_point: PathBuf,
        usage: Option<DiskUsage>,
    },
}

/// Finds all places on a new thread and sends one `PlacesMsg::Places`.
/// Disk usage is asked for separately with `start_usage`.
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
    };
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

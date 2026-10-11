//! Lists a folder on a background thread (PLAN.md section 4).
//!
//! Each listing gets its own thread, so a hung network mount only blocks
//! that thread and never the next navigation. Results carry the `generation`
//! they were started with; the UI ignores results from older generations.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::SystemTime;

use crate::entry::Entry;

/// Entries are sent in batches of this many, so big folders appear quickly.
pub const BATCH_SIZE: usize = 500;

/// Called after every message, so the UI can wake up (`ctx.request_repaint()`).
pub type Wake = Box<dyn Fn() + Send>;

#[derive(Debug)]
pub enum ListMsg {
    Batch {
        generation: u64,
        entries: Vec<Entry>,
    },
    /// The listing is complete. `skipped` counts entries that couldn't be read.
    /// `dir_mtime` is the folder's modified time from just before reading it;
    /// when it changes later, the listing is out of date.
    Finished {
        generation: u64,
        skipped: usize,
        dir_mtime: Option<SystemTime>,
    },
    /// The folder itself couldn't be opened (missing, permission denied, ...).
    Failed { generation: u64, error: io::Error },
}

/// Starts listing `dir` on a new thread. Messages go to `tx`.
pub fn start(dir: PathBuf, generation: u64, tx: Sender<ListMsg>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("listing".to_string())
        .spawn(move || list(dir, generation, tx, wake));
    if let Err(err) = spawned {
        crate::debug!("listing: could not start thread: {err}");
    }
}

fn list(dir: PathBuf, generation: u64, tx: Sender<ListMsg>, wake: Wake) {
    crate::debug!("listing: gen {generation}: {}", dir.display());
    // `send` fails only when the UI dropped its receiver. Then nobody is
    // listening, so we stop. That's what each `.is_err()` return below means.
    crate::debug_slow_fs();
    // Taken before reading, so a change during the listing still shows up
    // as a newer mtime on the next check.
    let dir_mtime = dir_modified(&dir);
    let items = match std::fs::read_dir(&dir) {
        Ok(items) => items,
        Err(error) => {
            let _ = tx.send(ListMsg::Failed { generation, error });
            wake();
            return;
        }
    };

    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut skipped = 0;
    for item in items {
        // One unreadable entry doesn't stop the listing; it's counted instead.
        let entry = match item.and_then(|item| Entry::from_dir_entry(&item)) {
            Ok(entry) => entry,
            Err(err) => {
                crate::debug!("listing: skipped an entry in {}: {err}", dir.display());
                skipped += 1;
                continue;
            }
        };
        batch.push(entry);

        if batch.len() == BATCH_SIZE {
            // mem::take swaps in an empty Vec and gives us the full one to send.
            let entries = std::mem::take(&mut batch);
            if tx
                .send(ListMsg::Batch {
                    generation,
                    entries,
                })
                .is_err()
            {
                return;
            }
            wake();
            crate::debug_slow_fs();
        }
    }

    if !batch.is_empty()
        && tx
            .send(ListMsg::Batch {
                generation,
                entries: batch,
            })
            .is_err()
    {
        return;
    }
    let _ = tx.send(ListMsg::Finished {
        generation,
        skipped,
        dir_mtime,
    });
    wake();
}

/// The folder's modified time. Adding, removing or renaming an entry changes
/// it; editing a file inside does not.
pub fn dir_modified(dir: &Path) -> Option<SystemTime> {
    crate::debug_slow_fs();
    let meta = std::fs::metadata(dir).ok()?;
    return meta.modified().ok();
}

/// The result of `start_mtime_check`.
#[derive(Debug)]
pub struct MtimeMsg {
    pub generation: u64,
    pub mtime: Option<SystemTime>,
}

/// Checks the folder's modified time on a new thread and sends it back with
/// `generation`. Used to notice changes while a folder is open (PLAN.md D9).
pub fn start_mtime_check(dir: PathBuf, generation: u64, tx: Sender<MtimeMsg>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("mtime-check".to_string())
        .spawn(move || {
            let mtime = dir_modified(&dir);
            if tx.send(MtimeMsg { generation, mtime }).is_ok() {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("listing: could not start mtime thread: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryKind;
    use crate::test_dir::TestDir;
    use std::ffi::OsString;
    use std::sync::mpsc;

    /// Runs a listing to completion and returns every message.
    fn run(dir: PathBuf, generation: u64) -> Vec<ListMsg> {
        let (tx, rx) = mpsc::channel();
        start(dir, generation, tx, Box::new(|| {}));
        // The loop ends when the worker drops its Sender.
        return rx.iter().collect();
    }

    fn all_entries(msgs: &[ListMsg]) -> Vec<Entry> {
        let mut all = Vec::new();
        for msg in msgs {
            if let ListMsg::Batch { entries, .. } = msg {
                all.extend(entries.iter().cloned());
            }
        }
        all.sort_by(|a, b| a.name.cmp(&b.name));
        return all;
    }

    #[test]
    fn lists_kinds_sizes_and_hidden() {
        let dir = TestDir::new("kinds");
        dir.file("a.txt", b"hello");
        dir.file(".secret", b"");
        dir.dir("sub");

        let msgs = run(dir.path(), 7);
        let entries = all_entries(&msgs);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, [".secret", "a.txt", "sub"]);

        assert_eq!(entries[0].kind, EntryKind::File);
        assert!(entries[0].hidden);
        assert_eq!(entries[1].size, 5);
        assert!(!entries[1].hidden);
        assert!(entries[1].mtime.is_some());
        assert_eq!(entries[2].kind, EntryKind::Dir);
        assert!(entries[2].is_dir());

        assert!(matches!(
            msgs.last(),
            Some(ListMsg::Finished {
                generation: 7,
                skipped: 0,
                dir_mtime: Some(_),
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks() {
        use std::os::unix::fs::symlink;
        let dir = TestDir::new("links");
        dir.file("target.txt", b"12345678");
        dir.dir("target_dir");
        symlink(dir.path().join("target.txt"), dir.path().join("to_file")).unwrap();
        symlink(dir.path().join("target_dir"), dir.path().join("to_dir")).unwrap();
        symlink(dir.path().join("missing"), dir.path().join("broken")).unwrap();

        let entries = all_entries(&run(dir.path(), 1));
        let kind_of = |name: &str| entries.iter().find(|e| e.name == name).unwrap().kind;
        assert_eq!(kind_of("to_file"), EntryKind::SymlinkToFile);
        assert_eq!(kind_of("to_dir"), EntryKind::SymlinkToDir);
        assert_eq!(kind_of("broken"), EntryKind::BrokenSymlink);
        let to_file = entries.iter().find(|e| e.name == "to_file").unwrap();
        assert_eq!(to_file.size, 8, "a symlink shows its target's size");
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("exec");
        dir.file("run.sh", b"#!/bin/sh\n");
        dir.file("plain.txt", b"");
        dir.dir("folder");
        let script = dir.path().join("run.sh");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let entries = all_entries(&run(dir.path(), 1));
        let executable: Vec<bool> = entries.iter().map(|e| e.executable).collect();
        // Sorted by name: folder, plain.txt, run.sh. Folders are never "executable".
        assert_eq!(executable, [false, false, true]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_names_survive() {
        use std::os::unix::ffi::OsStringExt;
        let dir = TestDir::new("bytes");
        let name = OsString::from_vec(b"bad\xffname".to_vec());
        std::fs::write(dir.path().join(&name), b"").unwrap();

        let entries = all_entries(&run(dir.path(), 1));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, name);
    }

    #[test]
    fn big_folders_come_in_batches() {
        let dir = TestDir::new("batches");
        for i in 0..1200 {
            dir.file(&format!("f{i}"), b"");
        }
        let msgs = run(dir.path(), 3);
        let mut sizes = Vec::new();
        for msg in &msgs {
            if let ListMsg::Batch {
                generation,
                entries,
            } = msg
            {
                assert_eq!(*generation, 3);
                sizes.push(entries.len());
            }
        }
        assert_eq!(sizes, [500, 500, 200]);
    }

    #[test]
    fn missing_folder_fails() {
        let dir = TestDir::new("missing");
        let msgs = run(dir.path().join("nope"), 9);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ListMsg::Failed { generation, error } => {
                assert_eq!(*generation, 9);
                assert_eq!(error.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn mtime_check_reports_back() {
        let dir = TestDir::new("mtime");
        let (tx, rx) = mpsc::channel();
        start_mtime_check(dir.path(), 4, tx, Box::new(|| {}));
        let msg = rx.recv().expect("one message");
        assert_eq!(msg.generation, 4);
        assert!(msg.mtime.is_some());
        assert_eq!(msg.mtime, dir_modified(&dir.path()));
    }

    #[test]
    fn empty_folder_finishes() {
        let dir = TestDir::new("empty");
        let msgs = run(dir.path(), 2);
        assert_eq!(msgs.len(), 1);
        assert!(matches!(
            msgs[0],
            ListMsg::Finished {
                generation: 2,
                skipped: 0,
                ..
            }
        ));
    }
}

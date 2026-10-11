//! Checks typed paths on a worker thread: does each exist, and is it a
//! folder? Optionally, can we write into a folder? The picker uses this
//! before accepting a typed name, since the UI thread can't touch the disk.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::listing::Wake;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    File,
    Folder,
}

#[derive(Debug)]
pub struct ProbeResult {
    /// Echoes the id given to `start`, so stale results can be ignored.
    pub id: u64,
    pub kinds: Vec<(PathBuf, PathKind)>,
    /// Whether `writable_check` (if given) can be written into.
    pub writable: Option<bool>,
}

pub fn kind_of(path: &Path) -> PathKind {
    crate::debug_slow_fs();
    // metadata() follows symlinks: a link to a folder counts as a folder.
    return match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => PathKind::Folder,
        Ok(_) => PathKind::File,
        Err(_) => PathKind::Missing,
    };
}

pub fn start(
    id: u64,
    paths: Vec<PathBuf>,
    writable_check: Option<PathBuf>,
    tx: Sender<ProbeResult>,
    wake: Wake,
) {
    let spawned = std::thread::Builder::new()
        .name("probe".to_string())
        .spawn(move || {
            let mut kinds = Vec::with_capacity(paths.len());
            for path in paths {
                let kind = kind_of(&path);
                kinds.push((path, kind));
            }
            let mut writable = None;
            if let Some(folder) = &writable_check {
                writable = Some(crate::platform::is_writable_dir(folder));
            }
            if tx
                .send(ProbeResult {
                    id,
                    kinds,
                    writable,
                })
                .is_ok()
            {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("probe: could not start thread: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn kinds_and_writable() {
        let dir = TestDir::new("probe");
        dir.file("f", b"");
        dir.dir("d");
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = vec![
            dir.path().join("f"),
            dir.path().join("d"),
            dir.path().join("nope"),
        ];
        start(7, paths, Some(dir.path()), tx, Box::new(|| {}));
        let result = rx.recv().unwrap();
        assert_eq!(result.id, 7);
        let kinds: Vec<PathKind> = result.kinds.iter().map(|(_, k)| *k).collect();
        assert_eq!(kinds, [PathKind::File, PathKind::Folder, PathKind::Missing]);
        assert_eq!(result.writable, Some(true));
    }
}

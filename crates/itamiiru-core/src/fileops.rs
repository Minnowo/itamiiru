//! File operations on a worker thread (PLAN.md section 4): copy, move,
//! trash, delete, rename and new folder.
//!
//! Each operation gets its own thread and its own channels. The worker
//! reports progress, failed items and conflicts. On a conflict it waits for
//! the UI's `Answer`. Cancelling sets an `AtomicBool` the worker checks
//! between files and between chunks of a file.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::listing::Wake;
use crate::names;

/// Folder trees deeper than this are not copied, to stay out of loops.
const MAX_DEPTH: usize = 256;
/// Files are copied in chunks of this size; cancel is checked between chunks.
const CHUNK_SIZE: usize = 1024 * 1024;
/// At most one progress message per this long.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub enum Op {
    Copy {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    Move {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    /// Permanently, without the trash.
    Delete {
        paths: Vec<PathBuf>,
    },
    Rename {
        from: PathBuf,
        new_name: OsString,
    },
    /// Creates "New folder", or "New folder (2)" and so on.
    NewFolder {
        parent: PathBuf,
    },
}

/// What to do when the destination already has an item with the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Replace,
    Skip,
    /// Keep both: the new one gets a numbered name, e.g. `a (2).txt`.
    KeepBoth,
    /// Stop the whole operation.
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct Answer {
    pub resolution: Resolution,
    /// Use the same resolution for every later conflict in this operation.
    pub apply_to_all: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// Top-level items (the ones the user selected).
    pub items_done: usize,
    pub items_total: usize,
    /// Only counted for copies; 0 otherwise.
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The file being worked on.
    pub current: PathBuf,
}

impl Progress {
    /// 0.0 to 1.0, by bytes when known, else by items.
    pub fn fraction(&self) -> f32 {
        if self.bytes_total > 0 {
            return (self.bytes_done as f64 / self.bytes_total as f64).min(1.0) as f32;
        }
        if self.items_total > 0 {
            return (self.items_done as f64 / self.items_total as f64).min(1.0) as f32;
        }
        return 0.0;
    }
}

#[derive(Debug)]
pub enum OpMsg {
    Progress(Progress),
    /// `existing` is in the way of `source`. The worker waits for an Answer.
    Conflict {
        source: PathBuf,
        existing: PathBuf,
    },
    /// One item failed; the operation goes on with the rest.
    ItemFailed {
        path: PathBuf,
        error: String,
    },
    /// Done. `created` lists the new top-level items, to select them.
    Finished {
        cancelled: bool,
        created: Vec<PathBuf>,
    },
}

/// The UI's side of a running operation.
pub struct OpHandle {
    pub messages: Receiver<OpMsg>,
    pub answers: Sender<Answer>,
    pub cancel: Arc<AtomicBool>,
}

pub fn start(op: Op, wake: Wake) -> OpHandle {
    let (msg_tx, msg_rx) = mpsc::channel();
    let (answer_tx, answer_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut worker = Worker {
        tx: msg_tx,
        answers: answer_rx,
        cancel: cancel.clone(),
        wake,
        progress: Progress::default(),
        last_report: Instant::now(),
        remembered: None,
        failures: 0,
        created: Vec::new(),
    };
    let spawned = std::thread::Builder::new()
        .name("file-op".to_string())
        .spawn(move || worker.run(op));
    if let Err(err) = spawned {
        crate::debug!("fileops: could not start thread: {err}");
    }
    return OpHandle {
        messages: msg_rx,
        answers: answer_tx,
        cancel,
    };
}

struct Worker {
    tx: Sender<OpMsg>,
    answers: Receiver<Answer>,
    cancel: Arc<AtomicBool>,
    wake: Wake,
    progress: Progress,
    last_report: Instant,
    /// An "apply to all" answer.
    remembered: Option<Resolution>,
    failures: usize,
    created: Vec<PathBuf>,
}

impl Worker {
    fn run(&mut self, op: Op) {
        crate::debug!("fileops: {op:?}");
        match op {
            Op::Copy { sources, dest_dir } => self.copy_or_move(&sources, &dest_dir, false),
            Op::Move { sources, dest_dir } => self.copy_or_move(&sources, &dest_dir, true),
            Op::Trash { paths } => self.remove_all(&paths, true),
            Op::Delete { paths } => self.remove_all(&paths, false),
            Op::Rename { from, new_name } => self.rename(&from, &new_name),
            Op::NewFolder { parent } => self.new_folder(&parent),
        }
        let created = std::mem::take(&mut self.created);
        let cancelled = self.cancelled();
        self.send(OpMsg::Finished { cancelled, created });
    }

    /// Sends a message and wakes the UI. If the UI is gone, cancels.
    fn send(&self, msg: OpMsg) {
        if self.tx.send(msg).is_err() {
            self.cancel.store(true, Ordering::Relaxed);
            return;
        }
        (self.wake)();
    }

    fn cancelled(&self) -> bool {
        return self.cancel.load(Ordering::Relaxed);
    }

    /// Sends progress, at most every PROGRESS_INTERVAL unless `force`.
    fn report(&mut self, force: bool) {
        if !force && self.last_report.elapsed() < PROGRESS_INTERVAL {
            return;
        }
        self.last_report = Instant::now();
        self.send(OpMsg::Progress(self.progress.clone()));
    }

    fn fail(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.failures += 1;
        crate::debug!("fileops: {}: {error}", path.display());
        self.send(OpMsg::ItemFailed {
            path: path.to_path_buf(),
            error: error.to_string(),
        });
    }

    /// Asks the UI what to do about a conflict, unless "apply to all" was chosen.
    fn ask(&mut self, source: &Path, existing: &Path) -> Resolution {
        if let Some(resolution) = self.remembered {
            return resolution;
        }
        self.send(OpMsg::Conflict {
            source: source.to_path_buf(),
            existing: existing.to_path_buf(),
        });
        // An error means the UI dropped its Sender: treat it as Cancel.
        let answer = match self.answers.recv() {
            Ok(answer) => answer,
            Err(_) => Answer {
                resolution: Resolution::Cancel,
                apply_to_all: false,
            },
        };
        if answer.apply_to_all {
            self.remembered = Some(answer.resolution);
        }
        if answer.resolution == Resolution::Cancel {
            self.cancel.store(true, Ordering::Relaxed);
        }
        return answer.resolution;
    }

    fn copy_or_move(&mut self, sources: &[PathBuf], dest_dir: &Path, is_move: bool) {
        self.progress.items_total = sources.len();
        if !is_move {
            // Total size first, for a progress bar by bytes.
            let mut total = 0;
            for source in sources {
                total += self.tree_size(source, 0);
            }
            self.progress.bytes_total = total;
        }
        self.report(true);

        for source in sources {
            if self.cancelled() {
                break;
            }
            self.progress.current = source.clone();
            self.copy_or_move_one(source, dest_dir, is_move);
            self.progress.items_done += 1;
            self.report(true);
        }
    }

    fn copy_or_move_one(&mut self, source: &Path, dest_dir: &Path, is_move: bool) {
        let Some(name) = source.file_name() else {
            self.fail(source, "can't copy this item");
            return;
        };
        let meta = match fs::symlink_metadata(source) {
            Ok(meta) => meta,
            Err(err) => {
                self.fail(source, err);
                return;
            }
        };
        let is_real_dir = meta.is_dir();
        if is_real_dir && dest_dir.starts_with(source) {
            self.fail(source, "can't copy or move a folder into itself");
            return;
        }

        let same_folder = source.parent() == Some(dest_dir);
        if same_folder && is_move {
            // Moving into the folder it's already in: nothing to do.
            return;
        }
        let target = if same_folder {
            // Pasting a copy next to the original: "a - Copy.txt".
            let Some(target) = self.free_name(dest_dir, name, !is_real_dir, true) else {
                self.fail(source, "no free name for the copy");
                return;
            };
            target
        } else {
            dest_dir.join(name)
        };

        let result = if is_move {
            self.move_entry(source, &target, 0)
        } else {
            self.copy_entry(source, &target, 0)
        };
        if let Some(final_path) = result {
            self.created.push(final_path);
        }
    }

    /// Finds a name that doesn't exist yet in `dir`: numbered ("a (2).txt")
    /// or, with `as_copy`, "a - Copy.txt", "a - Copy (2).txt".
    fn free_name(
        &self,
        dir: &Path,
        name: &std::ffi::OsStr,
        keep_extension: bool,
        as_copy: bool,
    ) -> Option<PathBuf> {
        let first = if as_copy { 1 } else { 2 };
        for n in first..10_000 {
            let candidate = if as_copy {
                names::copy_name(name, n, keep_extension)
            } else {
                names::numbered(name, n, keep_extension)
            };
            let path = dir.join(candidate);
            if fs::symlink_metadata(&path).is_err() {
                return Some(path);
            }
        }
        return None;
    }

    /// Decides where `source` goes when `target` may already exist.
    /// None means skip it (or the operation was cancelled).
    fn resolve_target(
        &mut self,
        source: &Path,
        source_is_dir: bool,
        target: &Path,
    ) -> Option<PathBuf> {
        let Ok(existing) = fs::symlink_metadata(target) else {
            return Some(target.to_path_buf());
        };
        if source_is_dir && existing.is_dir() {
            // Two folders with the same name are merged, like Explorer.
            // Conflicts inside are asked about one by one.
            return Some(target.to_path_buf());
        }
        match self.ask(source, target) {
            Resolution::Replace => {
                let removed = if existing.is_dir() {
                    fs::remove_dir_all(target)
                } else {
                    fs::remove_file(target)
                };
                if let Err(err) = removed {
                    self.fail(target, err);
                    return None;
                }
                return Some(target.to_path_buf());
            }
            Resolution::KeepBoth => {
                let dir = target.parent()?;
                let name = target.file_name()?;
                let free = self.free_name(dir, name, !source_is_dir, false);
                if free.is_none() {
                    self.fail(source, "no free name");
                }
                return free;
            }
            Resolution::Skip | Resolution::Cancel => return None,
        }
    }

    /// Copies a file, folder or symlink to `target`. Returns where it ended up.
    fn copy_entry(&mut self, source: &Path, target: &Path, depth: usize) -> Option<PathBuf> {
        if self.cancelled() {
            return None;
        }
        if depth > MAX_DEPTH {
            self.fail(source, "folders nested too deep");
            return None;
        }
        let meta = match fs::symlink_metadata(source) {
            Ok(meta) => meta,
            Err(err) => {
                self.fail(source, err);
                return None;
            }
        };
        let target = self.resolve_target(source, meta.is_dir(), target)?;

        if meta.file_type().is_symlink() {
            if let Err(err) = crate::platform::copy_symlink(source, &target) {
                self.fail(source, err);
                return None;
            }
            return Some(target);
        }

        if meta.is_dir() {
            if let Err(err) = fs::create_dir(&target)
                && err.kind() != io::ErrorKind::AlreadyExists
            {
                self.fail(&target, err);
                return None;
            }
            let children = match fs::read_dir(source) {
                Ok(children) => children,
                Err(err) => {
                    self.fail(source, err);
                    return Some(target);
                }
            };
            for child in children {
                if self.cancelled() {
                    break;
                }
                match child {
                    Ok(child) => {
                        let child_target = target.join(child.file_name());
                        self.copy_entry(&child.path(), &child_target, depth + 1);
                    }
                    Err(err) => self.fail(source, err),
                }
            }
            let _ = fs::set_permissions(&target, meta.permissions());
            return Some(target);
        }

        if meta.is_file() {
            self.progress.current = source.to_path_buf();
            if let Err(err) = self.copy_file(source, &target, &meta) {
                if err.kind() != io::ErrorKind::Interrupted {
                    self.fail(source, err);
                }
                return None;
            }
            return Some(target);
        }

        self.fail(
            source,
            "special files (devices, sockets, pipes) are skipped",
        );
        return None;
    }

    /// Copies one file in chunks, reporting progress. On cancel or error the
    /// partial copy is removed. Keeps permissions and the modified time.
    fn copy_file(&mut self, source: &Path, target: &Path, meta: &fs::Metadata) -> io::Result<()> {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        let mut buffer = vec![0u8; CHUNK_SIZE];
        loop {
            if self.cancelled() {
                drop(output);
                let _ = fs::remove_file(target);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let n = match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    drop(output);
                    let _ = fs::remove_file(target);
                    return Err(err);
                }
            };
            if let Err(err) = output.write_all(&buffer[..n]) {
                drop(output);
                let _ = fs::remove_file(target);
                return Err(err);
            }
            self.progress.bytes_done += n as u64;
            self.report(false);
        }
        let _ = output.set_permissions(meta.permissions());
        if let Ok(modified) = meta.modified() {
            let _ = output.set_modified(modified);
        }
        return Ok(());
    }

    /// Moves `source` to `target`: a rename on the same drive, a copy and
    /// delete across drives. Folders onto existing folders are merged.
    fn move_entry(&mut self, source: &Path, target: &Path, depth: usize) -> Option<PathBuf> {
        if self.cancelled() {
            return None;
        }
        if depth > MAX_DEPTH {
            self.fail(source, "folders nested too deep");
            return None;
        }
        let meta = match fs::symlink_metadata(source) {
            Ok(meta) => meta,
            Err(err) => {
                self.fail(source, err);
                return None;
            }
        };
        let target_existed = fs::symlink_metadata(target).is_ok();
        let target = self.resolve_target(source, meta.is_dir(), target)?;

        let merging = target_existed && meta.is_dir() && fs::symlink_metadata(&target).is_ok();
        if merging {
            let children = match fs::read_dir(source) {
                Ok(children) => children,
                Err(err) => {
                    self.fail(source, err);
                    return None;
                }
            };
            for child in children {
                if self.cancelled() {
                    break;
                }
                match child {
                    Ok(child) => {
                        let child_target = target.join(child.file_name());
                        self.move_entry(&child.path(), &child_target, depth + 1);
                    }
                    Err(err) => self.fail(source, err),
                }
            }
            // Fails, harmlessly, if something inside was skipped.
            let _ = fs::remove_dir(source);
            return Some(target);
        }

        match fs::rename(source, &target) {
            Ok(()) => return Some(target),
            Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
                // Another drive: copy, then delete the original only if the
                // whole copy worked.
                let failures_before = self.failures;
                let copied = self.copy_entry(source, &target, depth)?;
                if self.failures == failures_before && !self.cancelled() {
                    let removed = if meta.is_dir() {
                        fs::remove_dir_all(source)
                    } else {
                        fs::remove_file(source)
                    };
                    if let Err(err) = removed {
                        self.fail(source, err);
                    }
                }
                return Some(copied);
            }
            Err(err) => {
                self.fail(source, err);
                return None;
            }
        }
    }

    /// Total bytes of the files under `path` (symlinks not followed).
    fn tree_size(&self, path: &Path, depth: usize) -> u64 {
        if depth > MAX_DEPTH || self.cancelled() {
            return 0;
        }
        let Ok(meta) = fs::symlink_metadata(path) else {
            return 0;
        };
        if meta.is_file() {
            return meta.len();
        }
        if !meta.is_dir() {
            return 0;
        }
        let Ok(children) = fs::read_dir(path) else {
            return 0;
        };
        let mut total = 0;
        for child in children.flatten() {
            total += self.tree_size(&child.path(), depth + 1);
        }
        return total;
    }

    fn remove_all(&mut self, paths: &[PathBuf], to_trash: bool) {
        self.progress.items_total = paths.len();
        self.report(true);
        for path in paths {
            if self.cancelled() {
                break;
            }
            self.progress.current = path.clone();
            let result = if to_trash {
                crate::platform::trash(path)
            } else {
                delete_permanently(path)
            };
            if let Err(err) = result {
                self.fail(path, err);
            }
            self.progress.items_done += 1;
            self.report(false);
        }
    }

    fn rename(&mut self, from: &Path, new_name: &std::ffi::OsStr) {
        if let Err(message) = check_new_name(new_name) {
            self.fail(from, message);
            return;
        }
        let Some(parent) = from.parent() else {
            self.fail(from, "can't rename this item");
            return;
        };
        let target = parent.join(new_name);
        if target == from {
            return;
        }
        // Allow a case-only change ("a.txt" to "A.txt") on case-insensitive drives,
        // where the "existing" item is the same file.
        let exists = fs::symlink_metadata(&target).is_ok();
        let same_file_different_case =
            exists && from.as_os_str().eq_ignore_ascii_case(target.as_os_str());
        if exists && !same_file_different_case {
            self.fail(
                from,
                format!("\"{}\" already exists", new_name.to_string_lossy()),
            );
            return;
        }
        match fs::rename(from, &target) {
            Ok(()) => self.created.push(target),
            Err(err) => self.fail(from, err),
        }
    }

    fn new_folder(&mut self, parent: &Path) {
        let base = std::ffi::OsStr::new("New folder");
        for n in 1..10_000 {
            let name = if n == 1 {
                base.to_os_string()
            } else {
                names::numbered(base, n, false)
            };
            let path = parent.join(name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    self.created.push(path);
                    return;
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    self.fail(parent, err);
                    return;
                }
            }
        }
        self.fail(parent, "no free name for a new folder");
    }
}

/// Deletes a file, symlink or whole folder, without the trash.
/// A symlink to a folder removes only the link.
fn delete_permanently(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        return fs::remove_dir_all(path);
    }
    return fs::remove_file(path);
}

/// Rejects names that can't be a single file name.
pub fn check_new_name(name: &std::ffi::OsStr) -> Result<(), String> {
    let bytes = name.as_encoded_bytes();
    if bytes.is_empty() {
        return Err("the name can't be empty".to_string());
    }
    if name == "." || name == ".." {
        return Err("the name can't be \".\" or \"..\"".to_string());
    }
    for &b in bytes {
        if b == b'/' || b == 0 {
            return Err("the name can't contain \"/\"".to_string());
        }
        if cfg!(windows) && (b == b'\\' || b"<>:\"|?*".contains(&b)) {
            return Err("the name can't contain \\ / : * ? \" < > |".to_string());
        }
    }
    return Ok(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    /// Runs an operation to the end. Every conflict gets `answer`.
    /// Returns (failures, created, conflicts seen).
    fn run(op: Op, answer: Resolution, apply_to_all: bool) -> (Vec<String>, Vec<PathBuf>, usize) {
        let handle = start(op, Box::new(|| {}));
        let mut failures = Vec::new();
        let mut conflicts = 0;
        for msg in handle.messages.iter() {
            match msg {
                OpMsg::Conflict { .. } => {
                    conflicts += 1;
                    let _ = handle.answers.send(Answer {
                        resolution: answer,
                        apply_to_all,
                    });
                }
                OpMsg::ItemFailed { path, error } => {
                    failures.push(format!("{}: {error}", path.display()));
                }
                OpMsg::Finished { created, .. } => return (failures, created, conflicts),
                OpMsg::Progress(_) => {}
            }
        }
        panic!("no Finished message");
    }

    fn read(path: PathBuf) -> String {
        return fs::read_to_string(path).unwrap();
    }

    #[test]
    fn copy_files_and_folders() {
        let src = TestDir::new("copy-src");
        let dst = TestDir::new("copy-dst");
        src.file("a.txt", b"aaa");
        src.dir("sub");
        src.file("sub/b.txt", b"bbb");

        let sources = vec![src.path().join("a.txt"), src.path().join("sub")];
        let (failures, created, _) = run(
            Op::Copy {
                sources,
                dest_dir: dst.path(),
            },
            Resolution::Skip,
            false,
        );
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(created, [dst.path().join("a.txt"), dst.path().join("sub")]);
        assert_eq!(read(dst.path().join("sub/b.txt")), "bbb");
        assert!(src.path().join("a.txt").exists(), "copy keeps the original");
    }

    #[test]
    fn copy_into_same_folder_makes_copy_names() {
        let dir = TestDir::new("copy-same");
        dir.file("a.txt", b"x");
        for _ in 0..2 {
            let (failures, _, conflicts) = run(
                Op::Copy {
                    sources: vec![dir.path().join("a.txt")],
                    dest_dir: dir.path(),
                },
                Resolution::Skip,
                false,
            );
            assert!(failures.is_empty());
            assert_eq!(conflicts, 0);
        }
        assert!(dir.path().join("a - Copy.txt").exists());
        assert!(dir.path().join("a - Copy (2).txt").exists());
    }

    #[test]
    fn conflicts_skip_replace_keep_both() {
        let src = TestDir::new("conf-src");
        let dst = TestDir::new("conf-dst");
        src.file("a.txt", b"new");
        let copy = || Op::Copy {
            sources: vec![src.path().join("a.txt")],
            dest_dir: dst.path(),
        };

        dst.file("a.txt", b"old");
        let (_, created, conflicts) = run(copy(), Resolution::Skip, false);
        assert_eq!(conflicts, 1);
        assert!(created.is_empty());
        assert_eq!(read(dst.path().join("a.txt")), "old");

        run(copy(), Resolution::KeepBoth, false);
        assert_eq!(read(dst.path().join("a (2).txt")), "new");

        run(copy(), Resolution::Replace, false);
        assert_eq!(read(dst.path().join("a.txt")), "new");
    }

    #[test]
    fn apply_to_all_asks_once() {
        let src = TestDir::new("all-src");
        let dst = TestDir::new("all-dst");
        for name in ["1", "2", "3"] {
            src.file(name, b"new");
            dst.file(name, b"old");
        }
        let sources = vec![
            src.path().join("1"),
            src.path().join("2"),
            src.path().join("3"),
        ];
        let (_, _, conflicts) = run(
            Op::Copy {
                sources,
                dest_dir: dst.path(),
            },
            Resolution::Replace,
            true,
        );
        assert_eq!(conflicts, 1);
        assert_eq!(read(dst.path().join("3")), "new");
    }

    #[test]
    fn cancel_at_a_conflict_stops_everything() {
        let src = TestDir::new("cancel-src");
        let dst = TestDir::new("cancel-dst");
        src.file("1", b"new");
        src.file("2", b"new");
        dst.file("1", b"old");
        let sources = vec![src.path().join("1"), src.path().join("2")];
        run(
            Op::Copy {
                sources,
                dest_dir: dst.path(),
            },
            Resolution::Cancel,
            false,
        );
        assert!(!dst.path().join("2").exists());
    }

    #[test]
    fn folder_into_itself_is_refused() {
        let dir = TestDir::new("into-self");
        dir.dir("f");
        let (failures, _, _) = run(
            Op::Copy {
                sources: vec![dir.path().join("f")],
                dest_dir: dir.path().join("f"),
            },
            Resolution::Skip,
            false,
        );
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn move_and_merge_folders() {
        let src = TestDir::new("move-src");
        let dst = TestDir::new("move-dst");
        src.dir("f");
        src.file("f/new.txt", b"n");
        src.file("plain.txt", b"p");
        dst.dir("f");
        dst.file("f/old.txt", b"o");

        let sources = vec![src.path().join("f"), src.path().join("plain.txt")];
        let (failures, _, conflicts) = run(
            Op::Move {
                sources,
                dest_dir: dst.path(),
            },
            Resolution::Skip,
            false,
        );
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(conflicts, 0, "folders merge without asking");
        assert!(dst.path().join("f/old.txt").exists());
        assert!(dst.path().join("f/new.txt").exists());
        assert!(dst.path().join("plain.txt").exists());
        assert!(!src.path().join("f").exists());
        assert!(!src.path().join("plain.txt").exists());
    }

    #[test]
    fn delete_rename_new_folder() {
        let dir = TestDir::new("misc");
        dir.file("a.txt", b"");
        dir.dir("tree");
        dir.file("tree/x", b"");

        let (failures, created, _) = run(
            Op::Rename {
                from: dir.path().join("a.txt"),
                new_name: "b.txt".into(),
            },
            Resolution::Skip,
            false,
        );
        assert!(failures.is_empty());
        assert_eq!(created, [dir.path().join("b.txt")]);

        let (failures, _, _) = run(
            Op::Rename {
                from: dir.path().join("b.txt"),
                new_name: "tree".into(),
            },
            Resolution::Skip,
            false,
        );
        assert_eq!(failures.len(), 1, "rename onto an existing name fails");

        for expected in ["New folder", "New folder (2)"] {
            let (_, created, _) = run(
                Op::NewFolder { parent: dir.path() },
                Resolution::Skip,
                false,
            );
            assert_eq!(created, [dir.path().join(expected)]);
        }

        let paths = vec![dir.path().join("tree"), dir.path().join("b.txt")];
        let (failures, _, _) = run(Op::Delete { paths }, Resolution::Skip, false);
        assert!(failures.is_empty());
        assert!(!dir.path().join("tree").exists());
        assert!(!dir.path().join("b.txt").exists());
    }

    #[test]
    fn new_names_are_checked() {
        assert!(check_new_name("ok.txt".as_ref()).is_ok());
        assert!(check_new_name("".as_ref()).is_err());
        assert!(check_new_name("..".as_ref()).is_err());
        assert!(check_new_name("a/b".as_ref()).is_err());
    }

    #[test]
    fn progress_fraction() {
        let mut p = Progress::default();
        assert_eq!(p.fraction(), 0.0);
        p.items_total = 4;
        p.items_done = 1;
        assert_eq!(p.fraction(), 0.25);
        p.bytes_total = 100;
        p.bytes_done = 50;
        assert_eq!(p.fraction(), 0.5);
    }
}

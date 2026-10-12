//! Recursive name search on a worker thread.
//!
//! Walks breadth-first from a folder, so shallow matches come first, and
//! streams matches in batches. Symlinked folders aren't followed (no
//! loops), virtual filesystems like /proc are skipped, and the walk stops
//! at a result and depth limit or when cancelled.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::entry::{Entry, EntryKind};
use crate::listing::Wake;

/// Matches are sent at least this often, or when a batch is this big.
const BATCH_INTERVAL: Duration = Duration::from_millis(100);
const BATCH_SIZE: usize = 200;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Stop after this many matches.
    pub max_results: usize,
    /// Don't go more than this many folders below the start.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        return Limits {
            max_results: 10_000,
            max_depth: 64,
        };
    }
}

#[derive(Debug)]
pub enum SearchMsg {
    Batch {
        generation: u64,
        entries: Vec<Entry>,
    },
    /// Done. `truncated` means a limit was hit before the walk finished.
    Finished { generation: u64, truncated: bool },
}

/// What to search for, and how.
pub struct Query {
    pub root: PathBuf,
    pub text: String,
    pub show_hidden: bool,
    pub limits: Limits,
}

/// Starts a search on a new thread. Set the returned flag to cancel it.
pub fn start(query: Query, generation: u64, tx: Sender<SearchMsg>, wake: Wake) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_thread = cancel.clone();
    let spawned = std::thread::Builder::new()
        .name("search".to_string())
        .spawn(move || run(query, generation, &tx, &wake, &cancel_for_thread));
    if let Err(err) = spawned {
        crate::debug!("search: could not start thread: {err}");
    }
    return cancel;
}

/// Case-insensitive "name contains text". Non-UTF-8 names are compared
/// by their lossy text.
pub fn name_matches(name: &std::ffi::OsStr, lowercase_text: &str) -> bool {
    return name
        .to_string_lossy()
        .to_lowercase()
        .contains(lowercase_text);
}

fn run(query: Query, generation: u64, tx: &Sender<SearchMsg>, wake: &Wake, cancel: &AtomicBool) {
    crate::debug!("search: {:?} in {}", query.text, query.root.display());
    let needle = query.text.to_lowercase();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((query.root.clone(), 0));
    let mut batch = Vec::new();
    let mut found = 0;
    let mut truncated = false;
    let mut last_send = Instant::now();

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        crate::debug_slow_fs();
        let Ok(items) = std::fs::read_dir(&dir) else {
            // Unreadable folders (permissions) are skipped quietly.
            continue;
        };
        for item in items {
            if cancel.load(Ordering::Relaxed) {
                break 'walk;
            }
            let Ok(item) = item else {
                continue;
            };
            // The listing already says what kind each item is (no stat needed).
            // Only folders (to walk into) and matches need the full details,
            // which skips a stat for most files. Symlinks aren't folders here.
            let Ok(file_type) = item.file_type() else {
                continue;
            };
            let matches = name_matches(&item.file_name(), &needle);
            if !matches && !file_type.is_dir() {
                continue;
            }
            let Ok(entry) = Entry::from_dir_entry(&item) else {
                continue;
            };
            if entry.hidden && !query.show_hidden {
                continue;
            }
            // Only real folders are entered; symlinks to folders could loop.
            let descend = entry.kind == EntryKind::Dir
                && depth < query.limits.max_depth
                && !crate::platform::is_virtual_dir(&entry.path);
            if descend {
                queue.push_back((entry.path.clone(), depth + 1));
            } else if entry.kind == EntryKind::Dir && depth >= query.limits.max_depth {
                // A folder left unsearched because of the depth limit.
                truncated = true;
            }
            if matches {
                batch.push(entry);
                found += 1;
                if found >= query.limits.max_results {
                    truncated = true;
                    break 'walk;
                }
            }
        }
        if batch.len() >= BATCH_SIZE || (!batch.is_empty() && last_send.elapsed() >= BATCH_INTERVAL)
        {
            let entries = std::mem::take(&mut batch);
            if tx
                .send(SearchMsg::Batch {
                    generation,
                    entries,
                })
                .is_err()
            {
                return;
            }
            wake();
            last_send = Instant::now();
        }
    }
    // Folders still queued means the walk was stopped (limit or cancel).
    if !queue.is_empty() {
        truncated = true;
    }
    if !batch.is_empty()
        && tx
            .send(SearchMsg::Batch {
                generation,
                entries: batch,
            })
            .is_err()
    {
        return;
    }
    let _ = tx.send(SearchMsg::Finished {
        generation,
        truncated,
    });
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;
    use std::sync::mpsc;

    fn search(dir: &TestDir, text: &str, show_hidden: bool, limits: Limits) -> (Vec<String>, bool) {
        let (tx, rx) = mpsc::channel();
        let query = Query {
            root: dir.path(),
            text: text.to_string(),
            show_hidden,
            limits,
        };
        start(query, 1, tx, Box::new(|| {}));
        let mut names = Vec::new();
        for msg in rx.iter() {
            match msg {
                SearchMsg::Batch { entries, .. } => {
                    for entry in entries {
                        let relative = entry.path.strip_prefix(dir.path()).unwrap().to_path_buf();
                        names.push(relative.to_string_lossy().into_owned());
                    }
                }
                SearchMsg::Finished { truncated, .. } => {
                    names.sort();
                    return (names, truncated);
                }
            }
        }
        panic!("no Finished message");
    }

    fn tree() -> TestDir {
        let dir = TestDir::new("search");
        dir.file("Report.txt", b"");
        dir.dir("a");
        dir.file("a/old report.pdf", b"");
        dir.dir("a/b");
        dir.file("a/b/notes.md", b"");
        dir.file("a/b/REPORT-final.doc", b"");
        dir.dir(".hidden");
        dir.file(".hidden/report.txt", b"");
        return dir;
    }

    #[test]
    fn finds_matches_at_any_depth_ignoring_case() {
        let dir = tree();
        let (names, truncated) = search(&dir, "report", false, Limits::default());
        assert_eq!(
            names,
            ["Report.txt", "a/b/REPORT-final.doc", "a/old report.pdf"]
        );
        assert!(!truncated);
    }

    #[test]
    fn hidden_only_when_asked() {
        let dir = tree();
        let (names, _) = search(&dir, "report", true, Limits::default());
        assert!(
            names.contains(&".hidden/report.txt".to_string()),
            "{names:?}"
        );
    }

    #[test]
    fn limits_stop_the_walk() {
        let dir = tree();
        let few = Limits {
            max_results: 1,
            max_depth: 64,
        };
        let (names, truncated) = search(&dir, "report", false, few);
        assert_eq!(names.len(), 1);
        assert!(truncated);

        let shallow = Limits {
            max_results: 100,
            max_depth: 0,
        };
        let (names, truncated) = search(&dir, "report", false, shallow);
        assert_eq!(names, ["Report.txt"]);
        assert!(truncated, "folder a was not searched");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_are_not_followed() {
        let dir = tree();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("a/loop")).unwrap();
        let (names, _) = search(&dir, "notes", false, Limits::default());
        assert_eq!(names, ["a/b/notes.md"]);
    }

    #[test]
    fn matching() {
        assert!(name_matches("My File.TXT".as_ref(), "file.txt"));
        assert!(!name_matches("abc".as_ref(), "abd"));
        assert!(name_matches("anything".as_ref(), ""));
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Not a real test: `cargo test --release -p itamiiru-core -- --ignored --nocapture search_usr`
    #[test]
    #[ignore]
    fn search_usr() {
        let (tx, rx) = std::sync::mpsc::channel();
        let query = Query {
            root: PathBuf::from("/usr"),
            text: "python".to_string(),
            show_hidden: false,
            limits: Limits::default(),
        };
        let start_time = Instant::now();
        start(query, 1, tx, Box::new(|| {}));
        let mut count = 0;
        let mut first = None;
        for msg in rx.iter() {
            match msg {
                SearchMsg::Batch { entries, .. } => {
                    if first.is_none() {
                        first = Some(start_time.elapsed());
                    }
                    count += entries.len();
                }
                SearchMsg::Finished { truncated, .. } => {
                    println!(
                        "{count} results, first batch {:?}, total {:?}, truncated {truncated}",
                        first,
                        start_time.elapsed()
                    );
                    return;
                }
            }
        }
    }
}

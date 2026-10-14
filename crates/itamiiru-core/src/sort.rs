//! Sorting a listing, and the Type column's text.

use std::cmp::Ordering;
use std::path::Path;

use crate::entry::Entry;
use crate::natsort;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Modified,
    Created,
    Type,
    Size,
}

/// Which column the list is sorted by, and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortOrder {
    pub column: SortColumn,
    pub ascending: bool,
}

/// Sorts folders first (in either direction), then by `column`, then by name.
pub fn sort_entries(entries: &mut [Entry], column: SortColumn, ascending: bool) {
    entries.sort_by(|a, b| compare_entries(a, b, column, ascending));
}

pub fn compare_entries(a: &Entry, b: &Entry, column: SortColumn, ascending: bool) -> Ordering {
    // Folders always come first, even when the direction is reversed.
    if a.is_dir() != b.is_dir() {
        if a.is_dir() {
            return Ordering::Less;
        }
        return Ordering::Greater;
    }
    let ord = match column {
        SortColumn::Name => Ordering::Equal,
        SortColumn::Size => a.size.cmp(&b.size),
        SortColumn::Modified => a.mtime.cmp(&b.mtime),
        SortColumn::Created => a.created.cmp(&b.created),
        SortColumn::Type => compare_ignore_ascii_case(extension(a), extension(b)),
    };
    let ord = ord.then_with(|| natsort::compare(&a.name, &b.name));
    if ascending {
        return ord;
    }
    return ord.reverse();
}

/// Windows-style type text: "Folder", "PNG File", or "File" with no extension.
pub fn type_label(entry: &Entry) -> String {
    if entry.is_dir() {
        return "Folder".to_string();
    }
    let ext = extension(entry);
    if ext.is_empty() {
        return "File".to_string();
    }
    let ext = String::from_utf8_lossy(ext).to_uppercase();
    return format!("{ext} File");
}

/// The extension's bytes, or empty. `.bashrc` has no extension.
fn extension(entry: &Entry) -> &[u8] {
    return match Path::new(&entry.name).extension() {
        Some(ext) => ext.as_encoded_bytes(),
        None => &[],
    };
}

fn compare_ignore_ascii_case(a: &[u8], b: &[u8]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let ord = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
        if ord != Ordering::Equal {
            return ord;
        }
    }
    return a.len().cmp(&b.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryKind;
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, kind: EntryKind, size: u64, mtime_secs: u64) -> Entry {
        return Entry {
            name: name.into(),
            path: name.into(),
            kind,
            size,
            mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(mtime_secs)),
            created: None,
            hidden: false,
            executable: false,
        };
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        return entries.iter().map(|e| e.name.to_str().unwrap()).collect();
    }

    fn sample() -> Vec<Entry> {
        return vec![
            entry("b.txt", EntryKind::File, 300, 1),
            entry("zdir", EntryKind::Dir, 0, 5),
            entry("a10.png", EntryKind::File, 100, 3),
            entry("a2.PNG", EntryKind::File, 200, 2),
            entry("adir", EntryKind::SymlinkToDir, 0, 4),
        ];
    }

    #[test]
    fn by_name_folders_first() {
        let mut v = sample();
        sort_entries(&mut v, SortColumn::Name, true);
        assert_eq!(names(&v), ["adir", "zdir", "a2.PNG", "a10.png", "b.txt"]);
        sort_entries(&mut v, SortColumn::Name, false);
        assert_eq!(names(&v), ["zdir", "adir", "b.txt", "a10.png", "a2.PNG"]);
    }

    #[test]
    fn by_size_and_date() {
        let mut v = sample();
        sort_entries(&mut v, SortColumn::Size, true);
        assert_eq!(names(&v), ["adir", "zdir", "a10.png", "a2.PNG", "b.txt"]);
        sort_entries(&mut v, SortColumn::Modified, false);
        assert_eq!(names(&v), ["zdir", "adir", "a10.png", "a2.PNG", "b.txt"]);
    }

    #[test]
    fn by_date_created() {
        let mut v = sample();
        // Created in the opposite order to modified; unknown sorts first.
        for (i, e) in v.iter_mut().enumerate() {
            e.created = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(10 - i as u64));
        }
        v[3].created = None;
        sort_entries(&mut v, SortColumn::Created, true);
        assert_eq!(names(&v), ["adir", "zdir", "a2.PNG", "a10.png", "b.txt"]);
    }

    #[test]
    fn by_type_ignores_case_then_name() {
        let mut v = sample();
        sort_entries(&mut v, SortColumn::Type, true);
        assert_eq!(names(&v), ["adir", "zdir", "a2.PNG", "a10.png", "b.txt"]);
    }

    #[test]
    fn type_labels() {
        assert_eq!(
            type_label(&entry("x.png", EntryKind::File, 0, 0)),
            "PNG File"
        );
        assert_eq!(
            type_label(&entry("x.tar.gz", EntryKind::File, 0, 0)),
            "GZ File"
        );
        assert_eq!(
            type_label(&entry("Makefile", EntryKind::File, 0, 0)),
            "File"
        );
        assert_eq!(type_label(&entry(".bashrc", EntryKind::File, 0, 0)), "File");
        assert_eq!(
            type_label(&entry("photos.d", EntryKind::Dir, 0, 0)),
            "Folder"
        );
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::entry::EntryKind;

    /// Not a real test: `cargo test --release -p itamiiru-core -- --ignored --nocapture stream_sort`
    #[test]
    #[ignore]
    fn stream_sort_50k() {
        // Deterministic pseudo-random names like "file48213.txt".
        let mut seed: u64 = 12345;
        let mut all = Vec::new();
        for _ in 0..50_000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let name = format!("file{}.txt", seed >> 44);
            all.push(Entry {
                name: name.clone().into(),
                path: name.into(),
                kind: EntryKind::File,
                size: seed >> 40,
                mtime: None,
                created: None,
                hidden: false,
                executable: false,
            });
        }
        let start = std::time::Instant::now();
        let mut shown: Vec<Entry> = Vec::new();
        let mut worst = std::time::Duration::ZERO;
        for chunk in all.chunks(500) {
            let t = std::time::Instant::now();
            let mut batch = chunk.to_vec();
            sort_entries(&mut batch, SortColumn::Name, true);
            shown.append(&mut batch);
            sort_entries(&mut shown, SortColumn::Name, true);
            worst = worst.max(t.elapsed());
        }
        println!(
            "100 batches: total {:?}, worst batch {:?}",
            start.elapsed(),
            worst
        );
        let t = std::time::Instant::now();
        sort_entries(&mut shown, SortColumn::Size, false);
        println!("full re-sort by size: {:?}", t.elapsed());
    }
}

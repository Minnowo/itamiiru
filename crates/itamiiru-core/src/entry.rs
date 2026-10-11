//! One item in a folder listing.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    SymlinkToDir,
    SymlinkToFile,
    /// A symlink whose target is missing or can't be read.
    BrokenSymlink,
    /// Sockets, pipes, device files and anything else.
    Other,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: OsString,
    pub path: PathBuf,
    pub kind: EntryKind,
    /// In bytes. For a symlink this is the target's size; 0 for folders.
    pub size: u64,
    /// None when the OS can't tell us.
    pub mtime: Option<SystemTime>,
    pub hidden: bool,
    /// Can be run as a program (exec bit on Unix, extension on Windows).
    pub executable: bool,
}

impl Entry {
    /// True for folders and symlinks to folders: things you can open as a folder.
    pub fn is_dir(&self) -> bool {
        return self.kind == EntryKind::Dir || self.kind == EntryKind::SymlinkToDir;
    }

    /// Builds an entry from one `read_dir` item. Symlinks are followed for
    /// kind, size and mtime, like other file managers show them.
    pub fn from_dir_entry(item: &fs::DirEntry) -> io::Result<Entry> {
        let path = item.path();
        let name = item.file_name();
        // symlink_metadata describes the link itself, without following it.
        let link_meta = fs::symlink_metadata(&path)?;

        let (kind, meta) = if link_meta.file_type().is_symlink() {
            match fs::metadata(&path) {
                Ok(target) if target.is_dir() => (EntryKind::SymlinkToDir, target),
                Ok(target) => (EntryKind::SymlinkToFile, target),
                Err(_) => (EntryKind::BrokenSymlink, link_meta),
            }
        } else if link_meta.is_dir() {
            (EntryKind::Dir, link_meta)
        } else if link_meta.is_file() {
            (EntryKind::File, link_meta)
        } else {
            (EntryKind::Other, link_meta)
        };

        let size = if meta.is_dir() { 0 } else { meta.len() };
        let hidden = crate::platform::is_hidden(&name, &meta);
        let executable = crate::platform::is_executable(&name, &meta);
        return Ok(Entry {
            name,
            path,
            kind,
            size,
            mtime: meta.modified().ok(),
            hidden,
            executable,
        });
    }
}

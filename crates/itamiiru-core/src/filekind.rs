//! What kind of file an entry is, by its extension. Picks the painted icon.

use std::path::Path;

use crate::entry::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    Image,
    Archive,
    Text,
    Audio,
    Video,
    Document,
    Other,
}

const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff", "heic", "avif", "jxl",
];
const ARCHIVE: &[&str] = &[
    "zip", "tar", "gz", "tgz", "bz2", "xz", "txz", "zst", "7z", "rar", "lz", "lzma", "deb", "rpm",
    "iso", "jar", "apk",
];
const TEXT: &[&str] = &[
    "txt", "md", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "ini", "conf", "cfg",
    "rs", "go", "c", "h", "cpp", "hpp", "cc", "py", "js", "ts", "jsx", "tsx", "java", "kt", "rb",
    "lua", "sh", "bash", "zsh", "fish", "html", "htm", "css", "scss", "sql", "diff", "patch",
];
const AUDIO: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "wav", "m4a", "opus", "aac", "wma", "mid",
];
const VIDEO: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "m4v", "mpg", "mpeg", "ogv",
];
const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "epub",
];

pub fn file_kind(entry: &Entry) -> FileKind {
    if entry.is_dir() {
        return FileKind::Folder;
    }
    let Some(ext) = Path::new(&entry.name).extension() else {
        return FileKind::Other;
    };
    // Extensions in the tables are ASCII, so a non-UTF-8 one can't match.
    let Some(ext) = ext.to_str() else {
        return FileKind::Other;
    };
    let ext = ext.to_ascii_lowercase();
    let tables = [
        (IMAGE, FileKind::Image),
        (ARCHIVE, FileKind::Archive),
        (TEXT, FileKind::Text),
        (AUDIO, FileKind::Audio),
        (VIDEO, FileKind::Video),
        (DOCUMENT, FileKind::Document),
    ];
    for (table, kind) in tables {
        if table.contains(&ext.as_str()) {
            return kind;
        }
    }
    return FileKind::Other;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryKind;

    fn kind_of(name: &str, kind: EntryKind) -> FileKind {
        let entry = Entry {
            name: name.into(),
            path: name.into(),
            kind,
            size: 0,
            mtime: None,
            hidden: false,
            executable: false,
        };
        return file_kind(&entry);
    }

    #[test]
    fn by_extension() {
        assert_eq!(kind_of("a.PNG", EntryKind::File), FileKind::Image);
        assert_eq!(kind_of("a.tar.gz", EntryKind::File), FileKind::Archive);
        assert_eq!(kind_of("main.rs", EntryKind::File), FileKind::Text);
        assert_eq!(
            kind_of("song.flac", EntryKind::SymlinkToFile),
            FileKind::Audio
        );
        assert_eq!(kind_of("clip.mkv", EntryKind::File), FileKind::Video);
        assert_eq!(kind_of("paper.pdf", EntryKind::File), FileKind::Document);
    }

    #[test]
    fn others_and_folders() {
        assert_eq!(kind_of("Makefile", EntryKind::File), FileKind::Other);
        assert_eq!(kind_of(".bashrc", EntryKind::File), FileKind::Other);
        assert_eq!(kind_of("x.unknown", EntryKind::File), FileKind::Other);
        assert_eq!(kind_of("photos.png", EntryKind::Dir), FileKind::Folder);
        assert_eq!(kind_of("link", EntryKind::SymlinkToDir), FileKind::Folder);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_extension() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let entry = Entry {
            name: OsString::from_vec(b"a.\xff".to_vec()),
            path: "a".into(),
            kind: EntryKind::File,
            size: 0,
            mtime: None,
            hidden: false,
            executable: false,
        };
        assert_eq!(file_kind(&entry), FileKind::Other);
    }
}

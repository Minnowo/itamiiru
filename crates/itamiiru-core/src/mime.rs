//! Turning `mime:` filters into globs (P2.8).
//!
//! On Linux the shared-mime-info `globs2` files list `weight:type:glob`;
//! elsewhere (or if none are found) a small built-in table is used.
//! `image/*` style types match every type with that prefix.

use std::path::Path;

use crate::glob::Glob;

/// globs2 files bigger than this are skipped (the system one is ~100 KiB).
const MAX_FILE_SIZE: u64 = 8 * 1024 * 1024;

/// Common types, for systems without shared-mime-info.
const BUILT_IN: &[(&str, &[&str])] = &[
    ("text/plain", &["*.txt", "*.text", "*.log"]),
    ("text/markdown", &["*.md", "*.markdown"]),
    ("text/html", &["*.html", "*.htm"]),
    ("text/css", &["*.css"]),
    ("text/csv", &["*.csv"]),
    ("application/json", &["*.json"]),
    ("application/xml", &["*.xml"]),
    ("application/pdf", &["*.pdf"]),
    ("application/zip", &["*.zip"]),
    ("application/x-tar", &["*.tar"]),
    ("application/gzip", &["*.gz", "*.tgz"]),
    ("application/x-7z-compressed", &["*.7z"]),
    ("application/vnd.rar", &["*.rar"]),
    ("application/msword", &["*.doc"]),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        &["*.docx"],
    ),
    ("application/vnd.ms-excel", &["*.xls"]),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        &["*.xlsx"],
    ),
    ("application/vnd.oasis.opendocument.text", &["*.odt"]),
    ("image/png", &["*.png"]),
    ("image/jpeg", &["*.jpg", "*.jpeg", "*.jpe"]),
    ("image/gif", &["*.gif"]),
    ("image/webp", &["*.webp"]),
    ("image/bmp", &["*.bmp"]),
    ("image/svg+xml", &["*.svg"]),
    ("image/tiff", &["*.tif", "*.tiff"]),
    ("image/avif", &["*.avif"]),
    ("audio/mpeg", &["*.mp3"]),
    ("audio/flac", &["*.flac"]),
    ("audio/ogg", &["*.ogg", "*.oga", "*.opus"]),
    ("audio/x-wav", &["*.wav"]),
    ("audio/mp4", &["*.m4a"]),
    ("video/mp4", &["*.mp4", "*.m4v"]),
    ("video/x-matroska", &["*.mkv"]),
    ("video/webm", &["*.webm"]),
    ("video/quicktime", &["*.mov"]),
    ("video/x-msvideo", &["*.avi"]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    mime: String,
    glob: String,
    case_sensitive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MimeDb {
    entries: Vec<Entry>,
}

impl MimeDb {
    /// The system database where there is one, else the built-in table.
    /// Reads files: call at startup or on a worker.
    pub fn load() -> MimeDb {
        let mut db = MimeDb::default();
        for path in crate::platform::mime_glob_files() {
            if let Some(text) = read_small(&path) {
                db.add_globs2(&text);
            }
        }
        if db.entries.is_empty() {
            db = MimeDb::built_in();
        }
        return db;
    }

    pub fn built_in() -> MimeDb {
        let mut db = MimeDb::default();
        for (mime, globs) in BUILT_IN {
            for glob in *globs {
                db.entries.push(Entry {
                    mime: mime.to_string(),
                    glob: glob.to_string(),
                    case_sensitive: false,
                });
            }
        }
        return db;
    }

    /// Adds the lines of a globs2 file: `weight:type:glob[:flags]`.
    pub fn add_globs2(&mut self, text: &str) {
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() < 3 || parts[1].is_empty() || parts[2].is_empty() {
                continue;
            }
            let case_sensitive = parts.len() == 4 && parts[3].split(',').any(|flag| flag == "cs");
            self.entries.push(Entry {
                mime: parts[1].to_string(),
                glob: parts[2].to_string(),
                case_sensitive,
            });
        }
    }

    /// The globs for a type, or for every type under `major/` with `major/*`.
    pub fn globs_for(&self, mime: &str) -> Vec<Glob> {
        let prefix = mime.strip_suffix('*');
        let mut globs = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for entry in &self.entries {
            let wanted = match prefix {
                Some(prefix) => entry.mime.starts_with(prefix),
                None => entry.mime == mime,
            };
            if !wanted || seen.contains(&entry.glob.as_str()) {
                continue;
            }
            seen.push(&entry.glob);
            if entry.case_sensitive {
                globs.push(Glob::new(&entry.glob));
            } else {
                globs.push(Glob::case_insensitive(&entry.glob));
            }
        }
        return globs;
    }
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_SIZE {
        return None;
    }
    return std::fs::read_to_string(path).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOBS2: &str = "\
# comment
50:image/png:*.png
50:image/jpeg:*.jpg
50:image/jpeg:*.jpeg
50:text/x-makefile:Makefile:cs
50:text/plain:*.txt
40:image/png:*.png
broken line
";

    #[test]
    fn parses_globs2() {
        let mut db = MimeDb::default();
        db.add_globs2(GLOBS2);
        let png = db.globs_for("image/png");
        assert_eq!(png.len(), 1, "duplicates collapse");
        assert!(png[0].matches("photo.PNG"), "case-insensitive by default");
        let images = db.globs_for("image/*");
        assert_eq!(images.len(), 3);
        let make = db.globs_for("text/x-makefile");
        assert!(make[0].matches("Makefile"));
        assert!(!make[0].matches("makefile"), ":cs means case-sensitive");
        assert!(db.globs_for("video/mp4").is_empty());
    }

    #[test]
    fn built_in_table() {
        let db = MimeDb::built_in();
        let webp = db.globs_for("image/webp");
        assert!(webp[0].matches("a.WEBP"));
        assert!(db.globs_for("audio/*").len() >= 5);
    }

    #[test]
    fn load_finds_something() {
        // Either the system database or the built-in table.
        assert!(!MimeDb::load().globs_for("image/png").is_empty());
    }
}

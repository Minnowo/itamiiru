//! Reads the start of a file for the preview pane, on a worker
//! thread. Only the first `MAX_BYTES` are read, and only UTF-8 text without
//! NUL bytes counts as text; anything else gets no text preview.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::listing::Wake;

/// How much of the file to read.
pub const MAX_BYTES: usize = 16 * 1024;

pub struct TextPreview {
    pub path: PathBuf,
    /// None when the file isn't text or can't be read.
    pub text: Option<String>,
    /// The file goes on past what was read.
    pub truncated: bool,
}

/// Reads `path` on a new thread and sends the result to `tx`.
pub fn start(path: PathBuf, tx: Sender<TextPreview>, wake: Wake) {
    let spawned = std::thread::Builder::new()
        .name("text-preview".to_string())
        .spawn(move || {
            let (text, truncated) = match read(&path) {
                Some((text, truncated)) => (Some(text), truncated),
                None => (None, false),
            };
            if tx
                .send(TextPreview {
                    path,
                    text,
                    truncated,
                })
                .is_ok()
            {
                wake();
            }
        });
    if let Err(err) = spawned {
        crate::debug!("text preview: could not start thread: {err}");
    }
}

/// The start of the file as text, and whether there's more.
pub fn read(path: &Path) -> Option<(String, bool)> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(MAX_BYTES + 1);
    // One byte more than shown tells us whether the file goes on.
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let truncated = bytes.len() > MAX_BYTES;
    bytes.truncate(MAX_BYTES);
    let text = as_text(&bytes, truncated)?;
    return Some((text, truncated));
}

/// The bytes as text, or None if they look binary. `cut` says the bytes
/// were cut off, so a character split at the very end is fine.
pub fn as_text(bytes: &[u8], cut: bool) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            // error_len() is None when the input just ended mid-character.
            if !cut || err.error_len().is_some() {
                return None;
            }
            std::str::from_utf8(&bytes[..err.valid_up_to()]).ok()?
        }
    };
    // Windows line endings would show as stray boxes.
    return Some(text.replace("\r\n", "\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn text_and_binary() {
        assert_eq!(
            as_text(b"hello\r\nworld", false),
            Some("hello\nworld".to_string())
        );
        assert_eq!(
            as_text(b"caf\xc3\xa9", false),
            Some("caf\u{e9}".to_string())
        );
        assert_eq!(as_text(b"ELF\x00\x01", false), None);
        // Latin-1, not UTF-8.
        assert_eq!(as_text(b"caf\xe9 au lait", false), None);
        assert_eq!(as_text(b"", false), Some(String::new()));
    }

    #[test]
    fn character_cut_at_the_end() {
        // The first byte of a two-byte character: fine if we cut there.
        assert_eq!(as_text(b"abc\xc3", true), Some("abc".to_string()));
        assert_eq!(as_text(b"abc\xc3", false), None);
    }

    #[test]
    fn reads_only_the_start() {
        let dir = TestDir::new("textpreview");
        let long = "line\n".repeat(10_000);
        dir.file("long.txt", long.as_bytes());
        dir.file("short.txt", b"short");
        let (text, truncated) = read(&dir.path().join("long.txt")).expect("long");
        assert_eq!(text.len(), MAX_BYTES);
        assert!(truncated);
        let (text, truncated) = read(&dir.path().join("short.txt")).expect("short");
        assert_eq!(text, "short");
        assert!(!truncated);
        assert!(read(&dir.path().join("missing.txt")).is_none());
    }
}

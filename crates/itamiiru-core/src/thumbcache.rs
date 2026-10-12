//! The shared thumbnail cache (freedesktop thumbnail spec, PLAN.md section 7).
//!
//! `<root>/{normal,large,x-large,xx-large}/<md5 of URI>.png`, where root is
//! `~/.cache/thumbnails` on Linux. Nautilus, Dolphin and others use the same
//! files, so thumbnails they made show up here and the other way round.
//! A cached file counts only if its `Thumb::URI` and `Thumb::MTime` text
//! chunks match the file. Failed attempts go in `fail/itamiiru-<version>/`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::rgba::RgbaImage;

/// The spec's sizes, by the longest side in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThumbSize {
    Normal,
    Large,
    XLarge,
    XXLarge,
}

pub const ALL_SIZES: [ThumbSize; 4] = [
    ThumbSize::Normal,
    ThumbSize::Large,
    ThumbSize::XLarge,
    ThumbSize::XXLarge,
];

impl ThumbSize {
    pub fn pixels(self) -> u32 {
        return match self {
            ThumbSize::Normal => 128,
            ThumbSize::Large => 256,
            ThumbSize::XLarge => 512,
            ThumbSize::XXLarge => 1024,
        };
    }

    fn dir_name(self) -> &'static str {
        return match self {
            ThumbSize::Normal => "normal",
            ThumbSize::Large => "large",
            ThumbSize::XLarge => "x-large",
            ThumbSize::XXLarge => "xx-large",
        };
    }

    /// The smallest size at least `pixels` big (the biggest if none is).
    pub fn for_pixels(pixels: f32) -> ThumbSize {
        for size in ALL_SIZES {
            if size.pixels() as f32 >= pixels {
                return size;
            }
        }
        return ThumbSize::XXLarge;
    }
}

/// Cached thumbnails are never bigger than this many bytes; another app
/// could have written something hostile there.
const MAX_CACHED_FILE: u64 = 16 * 1024 * 1024;

pub struct ThumbCache {
    root: PathBuf,
}

impl ThumbCache {
    pub fn new(root: PathBuf) -> ThumbCache {
        return ThumbCache { root };
    }

    /// The cache in its usual place for this OS.
    pub fn open_default() -> Option<ThumbCache> {
        return Some(ThumbCache::new(crate::platform::thumbnail_dir()?));
    }

    /// True for files inside the cache itself, which must never be thumbnailed.
    pub fn contains(&self, path: &Path) -> bool {
        return path.starts_with(&self.root);
    }

    fn path_for(&self, size: ThumbSize, uri: &str) -> PathBuf {
        return self.root.join(size.dir_name()).join(file_name(uri));
    }

    fn fail_dir(&self) -> PathBuf {
        let app = format!("itamiiru-{}", env!("CARGO_PKG_VERSION"));
        return self.root.join("fail").join(app);
    }

    /// The cached thumbnail of this size, if there is one and it's for this
    /// version of the file. Shrunk to fit `size` if it's bigger.
    pub fn load(&self, size: ThumbSize, uri: &str, mtime: i64) -> Option<RgbaImage> {
        let data = read_capped(&self.path_for(size, uri), MAX_CACHED_FILE)?;
        let (image, text) = crate::imagefile::decode_png(&data, size.pixels()).ok()?;
        if !text_matches(&text, uri, mtime) {
            return None;
        }
        return Some(image);
    }

    /// Saves a thumbnail: written to a temporary file, then renamed, so other
    /// programs never see half a file.
    pub fn save(
        &self,
        size: ThumbSize,
        uri: &str,
        mtime: i64,
        file_size: u64,
        image: &RgbaImage,
    ) -> io::Result<()> {
        let text = [
            ("Thumb::URI", uri.to_string()),
            ("Thumb::MTime", mtime.to_string()),
            ("Thumb::Size", file_size.to_string()),
            ("Software", "itamiiru".to_string()),
        ];
        let png = encode_png(image, &text)?;
        return write_atomic(&self.path_for(size, uri), &png);
    }

    /// True if an earlier attempt on this version of the file failed.
    pub fn has_failed(&self, uri: &str, mtime: i64) -> bool {
        let path = self.fail_dir().join(file_name(uri));
        let Some(data) = read_capped(&path, MAX_CACHED_FILE) else {
            return false;
        };
        let Ok((_, text)) = crate::imagefile::decode_png(&data, 1) else {
            return false;
        };
        return text_matches(&text, uri, mtime);
    }

    /// Remembers that this version of the file can't be thumbnailed: an
    /// empty 1x1 PNG carrying the URI and mtime, as the spec asks.
    pub fn save_failure(&self, uri: &str, mtime: i64) -> io::Result<()> {
        let text = [
            ("Thumb::URI", uri.to_string()),
            ("Thumb::MTime", mtime.to_string()),
            ("Software", "itamiiru".to_string()),
        ];
        let png = encode_png(&RgbaImage::new(1, 1), &text)?;
        return write_atomic(&self.fail_dir().join(file_name(uri)), &png);
    }
}

/// The file's modified time in whole seconds, as `Thumb::MTime` stores it.
pub fn mtime_seconds(mtime: SystemTime) -> i64 {
    return match mtime.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    };
}

/// The `file://` URI the cache is keyed on, escaped the way GLib does it, so
/// the MD5 matches what Nautilus and other GTK programs computed.
pub fn file_uri(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut out = String::with_capacity(bytes.len() + 7);
    out.push_str("file://");
    for &b in bytes {
        // GLib's g_filename_to_uri leaves these alone in a path.
        let keep = b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    return out;
}

fn file_name(uri: &str) -> String {
    return format!("{}.png", crate::md5::hex_digest(uri.as_bytes()));
}

/// Checks the spec's two required text chunks. `Thumb::MTime` may have a
/// fraction (newer GLib writes `1756380786.000000`); only the seconds count.
fn text_matches(text: &[(String, String)], uri: &str, mtime: i64) -> bool {
    let mut uri_ok = false;
    let mut mtime_ok = false;
    for (key, value) in text {
        if key == "Thumb::URI" {
            uri_ok = value == uri;
        } else if key == "Thumb::MTime" {
            let seconds = value.split('.').next().unwrap_or("");
            mtime_ok = seconds.trim().parse::<i64>() == Ok(mtime);
        }
    }
    return uri_ok && mtime_ok;
}

/// Reads a whole file, or None if it's missing, unreadable or over `max`.
fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut data = Vec::new();
    // take(max + 1) reads at most one byte too many, which tells us it's too big.
    file.take(max + 1).read_to_end(&mut data).ok()?;
    if data.len() as u64 > max {
        return None;
    }
    return Some(data);
}

fn encode_png(image: &RgbaImage, text: &[(&str, String)]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // Thumbnails are small; spend the time on the next one, not on bytes.
    encoder.set_compression(png::Compression::Fast);
    for (key, value) in text {
        encoder
            .add_text_chunk(key.to_string(), value.clone())
            .map_err(io::Error::other)?;
    }
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer
        .write_image_data(&image.pixels)
        .map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    return Ok(out);
}

/// Writes `data` to a temporary file next to `path` (private to this user),
/// then renames it into place. Creates the folders if needed.
fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let Some(dir) = path.parent() else {
        return Err(io::Error::other("no parent folder"));
    };
    crate::platform::create_private_dir_all(dir)?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(".itamiiru-{}-{}.tmp", std::process::id(), n));
    let temp = dir.join(temp_name);

    let result = (|| {
        let mut file = crate::platform::create_private_file(&temp)?;
        file.write_all(data)?;
        return std::fs::rename(&temp, path);
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    return result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    fn test_image() -> RgbaImage {
        let mut image = RgbaImage::new(4, 2);
        for pixel in image.pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[10, 20, 30, 255]);
        }
        return image;
    }

    #[test]
    fn uri_escaping_matches_glib() {
        assert_eq!(
            file_uri(Path::new("/home/me/a b/#1 [x]%.jpg")),
            "file:///home/me/a%20b/%231%20%5Bx%5D%25.jpg"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/it's+fine=(ok)@x,y;z?.png")),
            "file:///tmp/it's+fine=(ok)@x,y%3Bz%3F.png"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/caf\u{e9}.png")),
            "file:///tmp/caf%C3%A9.png"
        );
    }

    #[test]
    fn size_choice() {
        assert_eq!(ThumbSize::for_pixels(48.0), ThumbSize::Normal);
        assert_eq!(ThumbSize::for_pixels(128.0), ThumbSize::Normal);
        assert_eq!(ThumbSize::for_pixels(129.0), ThumbSize::Large);
        assert_eq!(ThumbSize::for_pixels(384.0), ThumbSize::XLarge);
        assert_eq!(ThumbSize::for_pixels(5000.0), ThumbSize::XXLarge);
    }

    #[test]
    fn save_then_load() {
        let dir = TestDir::new("thumbcache");
        let cache = ThumbCache::new(dir.path());
        let uri = "file:///photos/a.jpg";
        cache
            .save(ThumbSize::Large, uri, 1000, 5, &test_image())
            .expect("save");

        let name = format!("{}.png", crate::md5::hex_digest(uri.as_bytes()));
        let file = dir.path().join("large").join(&name);
        assert!(file.is_file());
        assert_eq!(cache.load(ThumbSize::Large, uri, 1000), Some(test_image()));
        // A different mtime or size misses.
        assert_eq!(cache.load(ThumbSize::Large, uri, 1001), None);
        assert_eq!(cache.load(ThumbSize::Normal, uri, 1000), None);
        // No temporary files are left behind.
        let count = std::fs::read_dir(dir.path().join("large"))
            .expect("dir")
            .count();
        assert_eq!(count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("thumbcache-perms");
        let cache = ThumbCache::new(dir.path().join("thumbnails"));
        let uri = "file:///p.png";
        cache
            .save(ThumbSize::Normal, uri, 1, 1, &test_image())
            .expect("save");
        let file = cache.path_for(ThumbSize::Normal, uri);
        let mode = std::fs::metadata(&file).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(dir.path().join("thumbnails"))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    #[test]
    fn wrong_uri_is_a_miss() {
        // Same file name (as if MD5 collided), different URI inside.
        let dir = TestDir::new("thumbcache-uri");
        let cache = ThumbCache::new(dir.path());
        let png = encode_png(
            &test_image(),
            &[
                ("Thumb::URI", "file:///other.jpg".to_string()),
                ("Thumb::MTime", "7".to_string()),
            ],
        )
        .expect("encode");
        write_atomic(&cache.path_for(ThumbSize::Normal, "file:///a.jpg"), &png).expect("write");
        assert_eq!(cache.load(ThumbSize::Normal, "file:///a.jpg", 7), None);
    }

    #[test]
    fn fractional_mtime_from_glib() {
        let text = vec![
            ("Thumb::URI".to_string(), "file:///x".to_string()),
            ("Thumb::MTime".to_string(), "1756380786.000000".to_string()),
        ];
        assert!(text_matches(&text, "file:///x", 1756380786));
        assert!(!text_matches(&text, "file:///x", 1756380787));
        assert!(!text_matches(&text[..1], "file:///x", 1756380786));
    }

    #[test]
    fn failures_are_remembered_per_mtime() {
        let dir = TestDir::new("thumbcache-fail");
        let cache = ThumbCache::new(dir.path());
        let uri = "file:///broken.jpg";
        assert!(!cache.has_failed(uri, 50));
        cache.save_failure(uri, 50).expect("save failure");
        assert!(cache.has_failed(uri, 50));
        // The file changed: try again.
        assert!(!cache.has_failed(uri, 51));
        let version_dir = format!("itamiiru-{}", env!("CARGO_PKG_VERSION"));
        assert!(dir.path().join("fail").join(version_dir).is_dir());
    }

    #[test]
    fn cache_folder_is_excluded() {
        let cache = ThumbCache::new(PathBuf::from("/home/me/.cache/thumbnails"));
        assert!(cache.contains(Path::new("/home/me/.cache/thumbnails/large/x.png")));
        assert!(!cache.contains(Path::new("/home/me/.cache/thumbnails2/x.png")));
        assert!(!cache.contains(Path::new("/home/me/Pictures/x.png")));
    }

    #[test]
    fn mtime_before_1970() {
        let t = UNIX_EPOCH - std::time::Duration::from_secs(5);
        assert_eq!(mtime_seconds(t), -5);
        assert_eq!(
            mtime_seconds(UNIX_EPOCH + std::time::Duration::from_secs(9)),
            9
        );
    }
}

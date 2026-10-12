//! Thumbnails in the icon views (P5.8), an optional feature (`thumbnails`
//! in the config, on by default; PLAN.md section 6).
//!
//! The views draw whatever texture is ready and the app then calls `want`
//! with the items on screen, which asks the worker pool for the missing
//! ones. Finished images become GPU textures in `receive`. At most
//! `MAX_TEXTURES` are kept; the ones unused the longest go first.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use eframe::egui;
use itamiiru_core::entry::{Entry, EntryKind};
use itamiiru_core::thumbcache::{ThumbCache, ThumbSize};
use itamiiru_core::thumbnail::{Job, Pool};

/// Bounds GPU memory: 512 thumbnails of 256 px are about 128 MB at most.
const MAX_TEXTURES: usize = 512;

/// One thumbnail: a file as it was listed, at one size. A changed file has
/// a new mtime, so it gets a new key and a fresh thumbnail.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    mtime: Option<SystemTime>,
    size: ThumbSize,
}

enum Slot {
    /// Asked for; waiting for the pool.
    Pending,
    /// No thumbnail for this one; keep the painted icon.
    Failed,
    Ready {
        texture: egui::TextureHandle,
        /// The `clock` value when it was last on screen.
        last_used: u64,
    },
}

pub struct Thumbs {
    /// None when the feature is turned off.
    pool: Option<Pool>,
    slots: HashMap<Key, Slot>,
    /// Counts frames (`receive` calls).
    clock: u64,
    ready: usize,
}

impl Thumbs {
    /// Uses the shared cache in its usual place.
    pub fn new(ctx: &egui::Context, enabled: bool) -> Thumbs {
        let cache = if enabled {
            ThumbCache::open_default()
        } else {
            None
        };
        return Thumbs::with_cache(ctx, enabled, cache);
    }

    fn with_cache(ctx: &egui::Context, enabled: bool, cache: Option<ThumbCache>) -> Thumbs {
        let pool = if enabled {
            let ctx = ctx.clone();
            let wake = Arc::new(move || ctx.request_repaint());
            Some(Pool::new(cache, wake))
        } else {
            None
        };
        return Thumbs {
            pool,
            slots: HashMap::new(),
            clock: 0,
            ready: 0,
        };
    }

    /// The texture for this entry, if it's ready.
    pub fn get(&self, entry: &Entry, size: ThumbSize) -> Option<&egui::TextureHandle> {
        let key = key_for(entry, size)?;
        return match self.slots.get(&key) {
            Some(Slot::Ready { texture, .. }) => Some(texture),
            _ => None,
        };
    }

    /// Called with the entries on screen, top-left first: asks for the
    /// ones not seen yet and marks the ready ones as recently used.
    pub fn want(&mut self, on_screen: &[&Entry], size: ThumbSize) {
        let Some(pool) = &self.pool else {
            return;
        };
        // The pool takes the newest job first, so push in reverse: the
        // top-left item ends up on top of the stack.
        for entry in on_screen.iter().rev() {
            let Some(key) = key_for(entry, size) else {
                continue;
            };
            match self.slots.get_mut(&key) {
                Some(Slot::Ready { last_used, .. }) => *last_used = self.clock,
                Some(_) => {}
                None => {
                    pool.request(Job {
                        path: key.path.clone(),
                        mtime: key.mtime,
                        file_size: entry.size,
                        size,
                    });
                    self.slots.insert(key, Slot::Pending);
                }
            }
        }
    }

    /// Turns finished thumbnails into textures. Call once per frame,
    /// before any `want`.
    pub fn receive(&mut self, ctx: &egui::Context) {
        let Some(pool) = &self.pool else {
            return;
        };
        self.clock += 1;
        while let Some(done) = pool.try_recv() {
            let key = Key {
                path: done.job.path,
                mtime: done.job.mtime,
                size: done.job.size,
            };
            let slot = match done.image {
                Some(image) => {
                    let size = [image.width as usize, image.height as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, &image.pixels);
                    // Mipmaps keep a 256 px thumbnail smooth when drawn at 48.
                    let options = egui::TextureOptions {
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..egui::TextureOptions::LINEAR
                    };
                    let name = key.path.to_string_lossy().into_owned();
                    let texture = ctx.load_texture(name, color, options);
                    self.ready += 1;
                    Slot::Ready {
                        texture,
                        last_used: self.clock,
                    }
                }
                None => Slot::Failed,
            };
            if let Some(Slot::Ready { .. }) = self.slots.insert(key, slot) {
                self.ready -= 1;
            }
        }
        self.evict();
    }

    /// Drops the least recently used textures when there are too many.
    /// Dropping a TextureHandle frees the GPU texture.
    fn evict(&mut self) {
        if self.ready <= MAX_TEXTURES {
            return;
        }
        let mut ages: Vec<(u64, Key)> = Vec::new();
        for (key, slot) in &self.slots {
            if let Slot::Ready { last_used, .. } = slot {
                ages.push((*last_used, key.clone()));
            }
        }
        ages.sort_by_key(|(last_used, _)| *last_used);
        // Free a batch, so this doesn't run again on the very next result.
        let target = MAX_TEXTURES * 7 / 8;
        for (last_used, key) in ages {
            // Never what was on screen last frame: it's about to be drawn again.
            if self.ready <= target || last_used + 1 >= self.clock {
                break;
            }
            self.slots.remove(&key);
            self.ready -= 1;
        }
    }

    /// A different folder is showing: drop the jobs that haven't started,
    /// and forget failures so they're checked again next time.
    pub fn navigated(&mut self) {
        let Some(pool) = &self.pool else {
            return;
        };
        for job in pool.clear() {
            let key = Key {
                path: job.path,
                mtime: job.mtime,
                size: job.size,
            };
            self.slots.remove(&key);
        }
        self.slots.retain(|_, slot| !matches!(slot, Slot::Failed));
    }
}

/// Only regular files (and links to them) can have thumbnails.
fn key_for(entry: &Entry, size: ThumbSize) -> Option<Key> {
    if entry.kind != EntryKind::File && entry.kind != EntryKind::SymlinkToFile {
        return None;
    }
    return Some(Key {
        path: entry.path.clone(),
        mtime: entry.mtime,
        size,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// An uncompressed 24-bit BMP, the simplest image file to write by hand.
    fn gray_bmp(width: u32, height: u32) -> Vec<u8> {
        let stride = (width * 3).div_ceil(4) * 4;
        let pixels = stride * height;
        let mut out = Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(54 + pixels).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.resize(out.len() + pixels as usize, 90);
        return out;
    }

    /// A temporary folder with two small BMPs and a text file.
    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Fixture {
            let dir = std::env::temp_dir()
                .join(format!("itamiiru-thumbs-{}-{label}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create");
            for name in ["a.bmp", "b.bmp"] {
                std::fs::write(dir.join(name), gray_bmp(300, 150)).expect("write");
            }
            std::fs::write(dir.join("notes.txt"), "text").expect("write");
            return Fixture { dir };
        }

        fn entries(&self) -> Vec<Entry> {
            let mut out = Vec::new();
            for item in std::fs::read_dir(&self.dir).expect("list") {
                let item = item.expect("item");
                if item.path().is_file() {
                    out.push(Entry::from_dir_entry(&item).expect("entry"));
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            return out;
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Calls `receive` until `done` says so, or fails after a few seconds.
    fn wait_for(thumbs: &mut Thumbs, ctx: &egui::Context, done: impl Fn(&Thumbs) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(thumbs) {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
            thumbs.receive(ctx);
        }
    }

    #[test]
    fn images_get_textures_and_others_do_not() {
        let fixture = Fixture::new("flow");
        let ctx = egui::Context::default();
        let cache = ThumbCache::new(fixture.dir.join("cache"));
        let mut thumbs = Thumbs::with_cache(&ctx, true, Some(cache));
        let entries = fixture.entries();
        let on_screen: Vec<&Entry> = entries.iter().collect();
        let size = ThumbSize::Normal;

        thumbs.want(&on_screen, size);
        // Asking twice doesn't queue the same files again.
        thumbs.want(&on_screen, size);
        assert_eq!(thumbs.slots.len(), 3);

        wait_for(&mut thumbs, &ctx, |t| t.ready == 2);
        let texture = thumbs.get(&entries[0], size).expect("a.bmp");
        assert_eq!(texture.size(), [128, 64]);
        assert!(thumbs.get(&entries[1], size).is_some());
        wait_for(&mut thumbs, &ctx, |t| {
            let failed = t.slots.values().filter(|s| matches!(s, Slot::Failed));
            return failed.count() == 1;
        });
        assert!(thumbs.get(&entries[2], size).is_none());
        // Another size is a different thumbnail.
        assert!(thumbs.get(&entries[0], ThumbSize::Large).is_none());

        // Navigating keeps finished textures but forgets failures.
        thumbs.navigated();
        assert_eq!(thumbs.slots.len(), 2);
        assert!(thumbs.get(&entries[0], size).is_some());
    }

    #[test]
    fn turned_off_does_nothing() {
        let fixture = Fixture::new("off");
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs::with_cache(&ctx, false, None);
        let entries = fixture.entries();
        let on_screen: Vec<&Entry> = entries.iter().collect();
        thumbs.want(&on_screen, ThumbSize::Normal);
        thumbs.receive(&ctx);
        assert!(thumbs.slots.is_empty());
        assert!(thumbs.get(&entries[0], ThumbSize::Normal).is_none());
    }
}

//! Makes thumbnails on a small pool of worker threads.
//!
//! The UI asks for the thumbnails of the items on screen. Jobs wait on a
//! stack, newest on top, so whatever the user scrolled to most recently
//! is made first. Navigating away clears the stack. Each job looks in the
//! shared cache first and only decodes the image when that misses.
//! The UI keeps track of what it already asked for, so the pool doesn't
//! need to dedup.

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use crate::imagefile::{self, ImageError};
use crate::resize::downscale;
use crate::rgba::RgbaImage;
use crate::thumbcache::{self, ALL_SIZES, ThumbCache, ThumbSize};

/// One thumbnail to make.
#[derive(Clone, Debug)]
pub struct Job {
    pub path: PathBuf,
    /// The file's modified time and size from the listing; they decide
    /// whether a cached thumbnail is still good.
    pub mtime: Option<SystemTime>,
    pub file_size: u64,
    pub size: ThumbSize,
}

/// A finished job. `image` is None when there's no thumbnail (not an
/// image, damaged, too big); the UI then keeps the painted icon.
pub struct Done {
    pub job: Job,
    pub image: Option<RgbaImage>,
}

/// Called after each result, so the UI wakes up (`ctx.request_repaint()`).
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The job stack, shared by the UI and the workers.
struct Shared {
    state: Mutex<State>,
    /// Signalled when a job is pushed or the pool closes.
    changed: Condvar,
}

struct State {
    jobs: Vec<Job>,
    closed: bool,
}

pub struct Pool {
    shared: Arc<Shared>,
    results: Receiver<Done>,
}

impl Pool {
    /// Starts the workers: one per core minus one (for the UI), 1 to 4.
    pub fn new(cache: Option<ThumbCache>, wake: Wake) -> Pool {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        let threads = cores.saturating_sub(1).clamp(1, 4);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs: Vec::new(),
                closed: false,
            }),
            changed: Condvar::new(),
        });
        // One cache value shared read-only by every worker.
        let cache = Arc::new(cache);
        let (tx, results) = mpsc::channel();
        for i in 0..threads {
            let shared = shared.clone();
            let cache = cache.clone();
            let tx = tx.clone();
            let wake = wake.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("thumbnail-{i}"))
                .spawn(move || worker(&shared, cache.as_ref().as_ref(), &tx, &wake));
            if let Err(err) = spawned {
                crate::debug!("thumbnail: could not start a worker: {err}");
            }
        }
        return Pool { shared, results };
    }

    /// Queues a job on top of the stack.
    pub fn request(&self, job: Job) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.jobs.push(job);
        }
        self.shared.changed.notify_one();
    }

    /// Drops every job that hasn't started. Returns them, so the UI can
    /// forget it asked.
    pub fn clear(&self) -> Vec<Job> {
        if let Ok(mut state) = self.shared.state.lock() {
            return std::mem::take(&mut state.jobs);
        }
        return Vec::new();
    }

    /// The next finished thumbnail, if any. Never blocks.
    pub fn try_recv(&self) -> Option<Done> {
        return self.results.try_recv().ok();
    }
}

impl Drop for Pool {
    /// Tells the workers to stop after their current job.
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            state.jobs.clear();
        }
        self.shared.changed.notify_all();
    }
}

fn worker(shared: &Shared, cache: Option<&ThumbCache>, tx: &Sender<Done>, wake: &Wake) {
    loop {
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            // wait() releases the lock while sleeping and takes it back on wake.
            while state.jobs.is_empty() && !state.closed {
                state = match shared.changed.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            if state.closed {
                return;
            }
            // Last in, first out: the newest request is what's on screen.
            match state.jobs.pop() {
                Some(job) => job,
                None => continue,
            }
        };
        // make() already guards the decoders; this also covers the cache
        // code, so a panic anywhere costs one thumbnail, not the thread.
        // A panic becomes "no thumbnail".
        let image = std::panic::catch_unwind(|| make(cache, &job)).unwrap_or(None);
        if tx.send(Done { job, image }).is_err() {
            return;
        }
        wake();
    }
}

/// The whole pipeline for one file: cache, failure list,
/// sniff, size check, decode, save.
pub fn make(cache: Option<&ThumbCache>, job: &Job) -> Option<RgbaImage> {
    let path = &job.path;
    if let Some(cache) = cache
        && cache.contains(path)
    {
        return None;
    }
    let uri = thumbcache::file_uri(path);
    let mtime = thumbcache::mtime_seconds(job.mtime?);
    let max = job.size.pixels();

    if let Some(cache) = cache {
        // The asked-for size, or a bigger one shrunk down: Nautilus often
        // only has "large".
        for size in ALL_SIZES {
            if size < job.size {
                continue;
            }
            if let Some(image) = cache.load(size, &uri, mtime) {
                return Some(downscale(&image, max));
            }
        }
        if cache.has_failed(&uri, mtime) {
            return None;
        }
    }

    // Read the start first: most files aren't images, and this decides it
    // without reading the rest.
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; imagefile::SNIFF_LEN];
    let head_len = read_up_to(&mut file, &mut head)?;
    imagefile::sniff(&head[..head_len])?;
    if job.file_size > imagefile::MAX_FILE_SIZE {
        return None;
    }
    let mut data = head[..head_len].to_vec();
    // The file may have grown since it was listed: stop one byte past the limit.
    let limit = imagefile::MAX_FILE_SIZE + 1 - head_len as u64;
    file.take(limit).read_to_end(&mut data).ok()?;

    let result = imagefile::thumbnail(&data, max);
    let Some(cache) = cache else {
        return result.ok();
    };
    match result {
        Ok(image) => {
            if let Err(err) = cache.save(job.size, &uri, mtime, data.len() as u64, &image) {
                crate::debug!("thumbnail: could not save {uri}: {err}");
            }
            return Some(image);
        }
        Err(err) => {
            crate::debug!("thumbnail: {}: {err:?}", path.display());
            // A file that sniffed as an image but won't decode: don't try
            // again until it changes.
            if err != ImageError::UnknownFormat
                && let Err(err) = cache.save_failure(&uri, mtime)
            {
                crate::debug!("thumbnail: could not record failure: {err}");
            }
            return None;
        }
    }
}

/// Fills as much of `buf` as the file has. Returns how much was read.
fn read_up_to(file: &mut std::fs::File, buf: &mut [u8]) -> Option<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    return Some(filled);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;
    use std::time::Duration;

    fn png_file(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        let pixels = vec![200u8; (width * height * 3) as usize];
        writer.write_image_data(&pixels).expect("data");
        writer.finish().expect("finish");
        return out;
    }

    fn job_for(path: PathBuf, size: ThumbSize) -> Job {
        let meta = std::fs::metadata(&path).expect("meta");
        return Job {
            path,
            mtime: meta.modified().ok(),
            file_size: meta.len(),
            size,
        };
    }

    #[test]
    fn makes_and_caches() {
        let dir = TestDir::new("thumbnail-make");
        dir.dir("cache");
        dir.file("big.png", &png_file(400, 200));
        let cache = ThumbCache::new(dir.path().join("cache"));
        let job = job_for(dir.path().join("big.png"), ThumbSize::Normal);

        let image = make(Some(&cache), &job).expect("thumbnail");
        assert_eq!((image.width, image.height), (128, 64));

        // It was saved, and the second time is served from the cache.
        let uri = thumbcache::file_uri(&job.path);
        let mtime = thumbcache::mtime_seconds(job.mtime.expect("mtime"));
        assert!(cache.load(ThumbSize::Normal, &uri, mtime).is_some());
        assert_eq!(make(Some(&cache), &job), Some(image));
    }

    #[test]
    fn bigger_cached_size_is_reused() {
        let dir = TestDir::new("thumbnail-bigger");
        dir.file("a.png", &png_file(600, 300));
        let cache = ThumbCache::new(dir.path().join("cache"));
        let large = job_for(dir.path().join("a.png"), ThumbSize::Large);
        make(Some(&cache), &large).expect("large");
        // Remove the source: a Normal request must be served from Large.
        std::fs::remove_file(&large.path).expect("remove");
        let normal = Job {
            size: ThumbSize::Normal,
            ..large
        };
        let image = make(Some(&cache), &normal).expect("from cache");
        assert_eq!((image.width, image.height), (128, 64));
    }

    #[test]
    fn non_images_and_broken_images() {
        let dir = TestDir::new("thumbnail-broken");
        dir.file("notes.txt", b"just some text");
        dir.file("broken.png", b"\x89PNG\r\n\x1a\nnot really");
        dir.file("empty.jpg", b"");
        let cache = ThumbCache::new(dir.path().join("cache"));
        for name in ["notes.txt", "broken.png", "empty.jpg"] {
            let job = job_for(dir.path().join(name), ThumbSize::Normal);
            assert_eq!(make(Some(&cache), &job), None, "{name}");
        }
        // Only the file that claimed to be an image is recorded as failed.
        let broken = job_for(dir.path().join("broken.png"), ThumbSize::Normal);
        let uri = thumbcache::file_uri(&broken.path);
        let mtime = thumbcache::mtime_seconds(broken.mtime.expect("mtime"));
        assert!(cache.has_failed(&uri, mtime));
        let notes = thumbcache::file_uri(&dir.path().join("notes.txt"));
        assert!(!cache.has_failed(&notes, mtime));
    }

    #[test]
    fn never_thumbnails_the_cache() {
        let dir = TestDir::new("thumbnail-self");
        dir.dir("cache");
        dir.file("cache/x.png", &png_file(10, 10));
        let cache = ThumbCache::new(dir.path().join("cache"));
        let job = job_for(dir.path().join("cache/x.png"), ThumbSize::Normal);
        assert_eq!(make(Some(&cache), &job), None);
        // Without a cache it's just a file.
        assert!(make(None, &job).is_some());
    }

    #[test]
    fn pool_delivers_results() {
        let dir = TestDir::new("thumbnail-pool");
        for i in 0..6 {
            dir.file(&format!("{i}.png"), &png_file(50, 50));
        }
        let pool = Pool::new(None, Arc::new(|| {}));
        for i in 0..6 {
            pool.request(job_for(
                dir.path().join(format!("{i}.png")),
                ThumbSize::Normal,
            ));
        }
        let mut got = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while got < 6 && std::time::Instant::now() < deadline {
            match pool.try_recv() {
                Some(done) => {
                    assert!(done.image.is_some());
                    got += 1;
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert_eq!(got, 6);
    }

    #[test]
    fn clear_returns_waiting_jobs() {
        // No workers can run these: the pool is built by hand with none.
        let pool = Pool {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    jobs: Vec::new(),
                    closed: false,
                }),
                changed: Condvar::new(),
            }),
            results: mpsc::channel().1,
        };
        let job = Job {
            path: PathBuf::from("/x.png"),
            mtime: None,
            file_size: 0,
            size: ThumbSize::Normal,
        };
        pool.request(job.clone());
        pool.request(job);
        assert_eq!(pool.clear().len(), 2);
        assert_eq!(pool.clear().len(), 0);
    }
}

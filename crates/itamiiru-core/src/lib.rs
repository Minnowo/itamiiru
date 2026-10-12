//! itamiiru-core: everything that isn't UI. No egui in here.

pub mod actions;
pub mod bmp;
pub mod config;
pub mod entry;
pub mod exif;
pub mod filekind;
pub mod fileops;
pub mod format;
pub mod glob;
pub mod imagefile;
pub mod listing;
pub mod md5;
pub mod mime;
pub mod names;
pub mod natsort;
pub mod pathnorm;
pub mod percent;
pub mod places;
pub mod platform;
pub mod probe;
pub mod protocol;
pub mod resize;
pub mod rgba;
pub mod search;
pub mod sort;
pub mod textpreview;
pub mod thumbcache;
pub mod thumbnail;

#[cfg(test)]
mod test_dir;

use std::sync::OnceLock;
use std::time::Duration;

/// True when the environment has `ITAMI_DEBUG=1`. Checked once, then cached.
pub fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = ENABLED.get_or_init(|| {
        let value = std::env::var_os("ITAMI_DEBUG").unwrap_or_default();
        return value == "1";
    });
    return *enabled;
}

/// Prints to stderr when `ITAMI_DEBUG=1`. Same arguments as `println!`.
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::debug_enabled() {
            eprintln!("[itamiiru] {}", format_args!($($arg)*));
        }
    };
}

/// Test hook (debug builds only): with `ITAMI_DEBUG_SLOW_FS=<ms>`, workers
/// sleep this long before opening a folder and before each further batch,
/// to prove the UI stays responsive on slow or hung drives.
/// Does nothing in release builds.
pub fn debug_slow_fs() {
    if cfg!(debug_assertions) {
        static DELAY_MS: OnceLock<u64> = OnceLock::new();
        let ms = *DELAY_MS.get_or_init(|| {
            let value = std::env::var("ITAMI_DEBUG_SLOW_FS").unwrap_or_default();
            return value.trim().parse().unwrap_or(0);
        });
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
}

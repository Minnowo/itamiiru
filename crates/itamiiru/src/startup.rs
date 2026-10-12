//! Startup timing (P9.5): with ITAMI_DEBUG=1, each stage of startup is
//! logged with the milliseconds since `main` began, e.g.
//! `startup:   41.2 ms  window ready`. Costs nothing otherwise.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Call first thing in `main`.
pub fn begin() {
    let _ = START.set(Instant::now());
}

/// Logs that `stage` was reached.
pub fn mark(stage: &str) {
    if let Some(start) = START.get() {
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        itamiiru_core::debug!("startup: {ms:7.1} ms  {stage}");
    }
}

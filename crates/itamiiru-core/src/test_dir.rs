//! A temporary folder for tests, deleted when dropped. Replaces the `tempfile` crate.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// Creates a new empty folder under the system temp dir.
    pub fn new(label: &str) -> TestDir {
        // Process id + counter keeps parallel tests and parallel runs apart.
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("itamiiru-test-{}-{}-{}", std::process::id(), n, label);
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test dir");
        return TestDir { path };
    }

    pub fn path(&self) -> PathBuf {
        return self.path.clone();
    }

    pub fn file(&self, name: &str, contents: &[u8]) {
        std::fs::write(self.path.join(name), contents).expect("write test file");
    }

    pub fn dir(&self, name: &str) {
        std::fs::create_dir(self.path.join(name)).expect("create test subdir");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

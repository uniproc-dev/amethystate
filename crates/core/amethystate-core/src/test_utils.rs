use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A store path in a directory of its own, taken away with everything a
/// backend wrote beside it when it goes out of scope.
///
/// The only way to get a path for a test, so that every path a test writes to
/// is one somebody takes away again. A bare path in the shared temporary
/// directory is not offered: nothing removes what is written to it, and that
/// directory belongs to the whole machine.
///
/// The directory is also what makes the cleanup cheap. Sweeping the temporary
/// directory for names starting with this one costs a scan of every file in
/// it, per fixture, so a test with two dozen fixtures would pay for whatever
/// else has collected there. One directory per fixture holds two or three
/// files and is removed whole.
///
/// Declare it before the store so the store drops first: a backend holding the
/// file open would otherwise keep the removal from landing on Windows.
pub struct TempPath(PathBuf);

impl TempPath {
    pub fn new(suffix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();

        let at = std::env::temp_dir().join(format!("amethystate-{suffix}-{pid}-{nanos}-{seq}"));
        let _ = std::fs::create_dir_all(&at);

        Self(at.join(format!("{suffix}.db")))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for TempPath {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for TempPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// From a reference and never by value: the fixture has to outlive whatever
/// took the path, and a conversion that consumed it would take the directory
/// away at the moment it was handed over.
impl From<&TempPath> for PathBuf {
    fn from(at: &TempPath) -> Self {
        at.0.clone()
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

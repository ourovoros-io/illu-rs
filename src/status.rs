use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static STATUS_PATH: OnceLock<PathBuf> = OnceLock::new();

pub const READY: &str = "ready";

/// Initialize the status file path. Call once at startup.
pub fn init(repo_path: &Path) {
    let path = repo_path.join(".illu/status");
    let _ = STATUS_PATH.set(path);
}

/// Write current status. Silently ignores errors — status
/// is best-effort and must never block the main work.
pub fn set(message: &str) {
    let Some(path) = STATUS_PATH.get() else {
        return;
    };
    let _ = std::fs::write(path, message);
}

/// Clear status (server idle / shutting down).
pub fn clear() {
    let Some(path) = STATUS_PATH.get() else {
        return;
    };
    let _ = std::fs::remove_file(path);
}

/// RAII guard that resets status to READY on drop.
/// Ensures status is cleared even on error paths.
pub struct StatusGuard;

impl StatusGuard {
    #[must_use]
    pub fn new(message: &str) -> Self {
        set(message);
        Self
    }
}

impl Drop for StatusGuard {
    fn drop(&mut self) {
        set(READY);
    }
}

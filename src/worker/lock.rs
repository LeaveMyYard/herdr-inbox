//! The worker's lock (Python `worker.hold_lock`, `is_running`): an `flock`
//! on `worker-<key>.lock`, so a Python and a Rust worker exclude each other.

use std::io;
use std::path::Path;
use std::time::Duration;

use herdr_agent_kit::internal::plugin_paths::inbox::WORKER_LOCK_RETRY;
use herdr_agent_kit::plugin_env::{HeldLock, LockFile};

/// How often [`hold_within`] retries.
const RETRY_POLL: Duration = Duration::from_millis(10);

/// The lock, or `None` if another worker holds it (Python `hold_lock`:
/// `LOCK_EX | LOCK_NB`, the directory created first). Dropping it releases.
pub fn hold(lock: &Path) -> io::Result<Option<HeldLock>> {
    if let Some(dir) = lock.parent() {
        std::fs::create_dir_all(dir)?;
    }
    LockFile::new(lock).try_exclusive()
}

/// Some worker holds `lock` (Python `is_running`: a shared probe, released
/// at once).
pub fn is_running(lock: &Path) -> bool {
    LockFile::new(lock).held_elsewhere()
}

/// The lock for a worker that starts (the Rust worker's contract with
/// Python `worker._start_rust_worker`): `LOCK_EX | LOCK_NB` retried for
/// [`WORKER_LOCK_RETRY`] before concluding another worker holds it, because
/// a Python probe holds `LOCK_SH` for an instant every 50 ms.
pub fn hold_within(lock: &Path) -> io::Result<Option<HeldLock>> {
    if let Some(dir) = lock.parent() {
        std::fs::create_dir_all(dir)?;
    }
    LockFile::new(lock).exclusive_within(WORKER_LOCK_RETRY, RETRY_POLL, || true)
}

#[cfg(test)]
mod tests;

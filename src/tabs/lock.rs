//! The tab-label lock (Python `tabs.marker_lock`): count, re-read and rename
//! run under a cross-process `flock` on `<db>.markers.lock`, never the
//! database's write lock, so a slow or wedged herdr never blocks questions,
//! drafts or answers.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use herdr_agent_kit::plugin_env::lock::{HeldLock, LockFile};

/// How long a label update waits for the lock (Python `tabs.LOCK_TIMEOUT`).
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
/// How often it tries meanwhile (Python's `time.sleep(0.05)`).
const POLL: Duration = Duration::from_millis(50);
/// What a skipped update says (Python's warning, word for word).
pub const BUSY: &str = "tab labels busy; skipping this update (the next change or sync fixes it)";

/// The lock file next to the database at `db`.
pub fn lock_path(db: &Path) -> PathBuf {
    let mut path = OsString::from(db.as_os_str());
    path.push(".markers.lock");
    PathBuf::from(path)
}

/// The lock, waiting up to `wait`; `None` (after `warn`ing [`BUSY`]) if
/// another process kept it that long. `Err`: the lock file cannot be opened.
pub fn marker_lock(
    db: &Path,
    wait: Duration,
    warn: &mut dyn FnMut(&str),
) -> io::Result<Option<HeldLock>> {
    let held = LockFile::new(lock_path(db)).exclusive_within(wait, POLL, || true)?;
    if held.is_none() {
        warn(BUSY);
    }
    Ok(held)
}

#[cfg(test)]
mod tests;

//! The spawn rate limit (Python `worker._may_spawn`): a worker that crashes
//! at start (a broken newer release) must not cost one process and one
//! `worker.log` entry per hook or tool call.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// `ensure` starts at most one worker per revision this often, per herdr
/// server. A legitimate start waits at most this long.
pub const ENSURE_EVERY: Duration = Duration::from_secs(5);

/// `<lock>.spawned-<revision>`, whose mtime is the last start.
pub fn stamp_path(lock: &Path, revision: i64) -> PathBuf {
    let mut name = lock.as_os_str().to_owned();
    name.push(format!(".spawned-{revision}"));
    PathBuf::from(name)
}

/// True, and stamped now, unless this revision started a worker within
/// [`ENSURE_EVERY`]. A stamp dated in the future does not count. Failing to
/// stamp does not stop the start.
pub fn may_spawn(lock: &Path, revision: i64) -> bool {
    let stamp = stamp_path(lock, revision);
    let age = fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .map(|m| SystemTime::now().duration_since(m));
    if matches!(age, Ok(Ok(a)) if a < ENSURE_EVERY) {
        return false;
    }
    if let Some(dir) = stamp.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&stamp) {
        let _ = file.set_modified(SystemTime::now());
    }
    true
}

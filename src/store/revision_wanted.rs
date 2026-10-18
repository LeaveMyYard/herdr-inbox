//! The "revision wanted" marker (herdr-inbox#50; Python `revision_wanted.py`,
//! same file, same format). A hook that finds the database not migrated to
//! its revision writes the revision it wants; the worker holding the lock
//! then starts the newer worker, which stands by and takes over (Python
//! `worker_handover.py`). A hook cannot start a worker itself.
//!
//! `<database>.revision-wanted` holds one line, `<revision> <unix seconds>`. It
//! counts while younger than [`REVISION_WANTED_TTL`] and not dated more than
//! [`CLOCK_SKEW`] ahead. Writes hold `<database>.revision-wanted.lock`
//! (flock): a fresher, higher revision is never lowered or extended.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use herdr_agent_kit::plugin_env::LockFile;

/// The marker's suffix after the database file name.
pub const SUFFIX: &str = ".revision-wanted";
/// Seconds a marker counts after its last write.
pub const REVISION_WANTED_TTL: f64 = 600.0;
/// Seconds a marker may be dated in the future (a clock step back).
pub const CLOCK_SKEW: f64 = 60.0;

/// The marker of database `db`.
pub fn path_for(db: &Path) -> PathBuf {
    let mut name = db.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(SUFFIX);
    db.with_file_name(name)
}

/// Now, in unix seconds.
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// The revision a newer release wants `db` at, or `None`.
pub fn wanted(db: &Path, now: f64) -> Option<i64> {
    let text = fs::read_to_string(path_for(db)).ok()?;
    let mut parts = text.split_whitespace();
    let revision: i64 = parts.next()?.parse().ok()?;
    let written: f64 = parts.next()?.parse().ok()?;
    let age = now - written;
    (parts.next().is_none() && (-CLOCK_SKEW..=REVISION_WANTED_TTL).contains(&age))
        .then_some(revision)
}

/// A process of `revision` found `db` not migrated to it: say so. Errors are
/// ignored (the next not-ready open writes again).
pub fn want(db: &Path, revision: i64, now: f64) {
    let path = path_for(db);
    let mut lock_name = path.clone().into_os_string();
    lock_name.push(".lock");
    let Ok(_held) = LockFile::new(PathBuf::from(lock_name)).exclusive() else {
        return;
    };
    if wanted(db, now).is_some_and(|current| current > revision) {
        return; // a fresher, higher request stands
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    if fs::write(&tmp, format!("{revision} {}\n", now as i64)).is_ok()
        && fs::rename(&tmp, &path).is_err()
    {
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests;

//! Reading and writing the holder and standby records (Python
//! `worker_handover.write_holder`, `clear_holder`, `holder_revision`,
//! `live_standby`).

use std::fs;
use std::path::Path;

use herdr_agent_kit::agent_procs::pid_alive;

use super::{STANDBY_FRESH, holder_path, names_me, standby_path, start_time};

/// Writes `text` to `path` through `.<name>.<pid>.tmp` and a rename, so a
/// reader sees the old record or the new one, never half of one. A failure
/// leaves no temporary file behind and is not reported (Python `_write`).
pub(super) fn write(path: &Path, text: &str) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    if fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, path))
        .is_err()
    {
        let _ = fs::remove_file(&tmp);
    }
}

/// This process holds `lock` and runs `revision`.
pub fn write_holder(lock: &Path, revision: i64) {
    let pid = std::process::id();
    write(
        &holder_path(lock),
        &format!("{revision} {pid} {}\n", start_time(pid)),
    );
}

/// This process lets go of `lock`: its record goes, never another's.
pub fn clear_holder(lock: &Path) {
    let path = holder_path(lock);
    if names_me(&path) {
        let _ = fs::remove_file(path);
    }
}

/// The revision of the worker holding `lock`: its record while that process
/// lives with the same start time (or an unreadable one, `-`), else 1 (a
/// worker from before the record).
pub fn holder_revision(lock: &Path) -> i64 {
    let read = || -> Option<i64> {
        let text = fs::read_to_string(holder_path(lock)).ok()?;
        let [revision, pid, started] = text.split_whitespace().collect::<Vec<_>>()[..] else {
            return None;
        };
        let (revision, pid): (i64, u32) = (revision.parse().ok()?, pid.parse().ok()?);
        let same = started == "-" || started == start_time(pid);
        (pid_alive(Some(pid)) && same).then_some(revision)
    };
    read().unwrap_or(1)
}

/// `(revision, pid)` of a worker standing by for `lock` at unix time `now`:
/// its heartbeat within [`STANDBY_FRESH`] seconds either way (a record
/// dated further ahead does not count), its process alive with the start
/// time it wrote (a reused pid is not the standby).
pub fn live_standby(lock: &Path, now: f64) -> Option<(i64, u32)> {
    let text = fs::read_to_string(standby_path(lock)).ok()?;
    let [revision, pid, beat, started] = text.split_whitespace().collect::<Vec<_>>()[..] else {
        return None;
    };
    let (revision, pid, beat): (i64, u32, f64) = (
        revision.parse().ok()?,
        pid.parse().ok()?,
        beat.parse().ok()?,
    );
    let age = now - beat;
    if !(-STANDBY_FRESH..=STANDBY_FRESH).contains(&age) || !pid_alive(Some(pid)) {
        return None;
    }
    if pid != std::process::id() && started != "-" && started != start_time(pid) {
        return None;
    }
    Some((revision, pid))
}

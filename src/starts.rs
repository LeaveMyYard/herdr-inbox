//! The SessionStart start markers (Python `store.record_start`): a Claude
//! process RESUMED a session, in a herdr pane.
//!
//! `<state_dir>/starts/<session>` holds the pane id, and its mtime is the
//! time. A file, not a column: no schema change, no database lock on the
//! hook's path. The delivery worker reads them (`store.live_session`): a
//! session resumed after its `continued-in` record runs itself again.

use std::fs;
use std::io;
use std::path::Path;

use herdr_agent_kit::agent_session::SessionId;

/// Records that `session_id` was resumed now in pane `pane_id`, under
/// `state_dir` (the caller passes the inbox's state directory,
/// `plugin_paths::inbox::state_dir`, and makes the resume-only check: only
/// SessionStart with source `resume` writes a marker). Nothing for
/// an id that is not a canonical session id (it names the file). The marker
/// is written to a per-process temporary file and renamed into place, so a
/// reader sees a whole file and every write gives a new mtime.
pub fn record_start(state_dir: &Path, session_id: &str, pane_id: Option<&str>) -> io::Result<()> {
    let Some(sid) = SessionId::parse(session_id) else {
        return Ok(());
    };
    let dir = state_dir.join("starts");
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".{sid}.{}.tmp", std::process::id()));
    fs::write(&tmp, pane_id.unwrap_or(""))?;
    fs::rename(&tmp, dir.join(sid.as_str()))
}

/// Drops the start markers under `state_dir` older than `keep` (Python
/// `store.prune_starts`, which the worker runs at most hourly); the number
/// dropped. A marker only matters while it is newer than its session's
/// `continued-in` record; a session idle longer than `keep` with its marker
/// gone falls back to D6 plus the typed-once path.
pub fn prune_starts(state_dir: &Path, keep: std::time::Duration) -> usize {
    let cutoff = std::time::SystemTime::now() - keep;
    let Ok(entries) = fs::read_dir(state_dir.join("starts")) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let old = fs::metadata(entry.path())
            .and_then(|m| m.modified())
            .is_ok_and(|m| m < cutoff);
        if old && fs::remove_file(entry.path()).is_ok() {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests;

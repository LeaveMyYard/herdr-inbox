//! The delivery hook's fast path (Python `hook._has_candidates`): is any
//! answer for this session waiting for a hook to carry it?

use std::path::Path;

use rusqlite::types::Value;

use super::MAX_HOOK_MISSES;
use super::busy::{PENDING_TIMEOUT, bounded};
use super::connect::read_only;
use super::family::{Continuations, family};

/// Is an answer of `session_id`'s family (see [`family`]) answered, neither
/// delivered nor seen, not at the hook-miss cap and its agent not gone? The
/// same filter as the hook's candidates, read-only and bounded by
/// [`PENDING_TIMEOUT`], so a session with nothing waiting never opens the
/// database for writing. `false` when the database is missing, locked or
/// unreadable.
pub fn has_candidates(path: &Path, session_id: &str, files: &impl Continuations) -> bool {
    if !path.exists() {
        return false;
    }
    let look = |db: &rusqlite::Connection| {
        let fam = family(db, session_id, files)?;
        let marks = vec!["?"; fam.len()].join(",");
        let sql = format!(
            "SELECT 1 FROM questions WHERE session_id IN ({marks}) AND status = 'answered' \
             AND delivered_at IS NULL AND acknowledged_at IS NULL AND id NOT IN (SELECT id FROM question_ext \
             WHERE COALESCE(hook_misses, 0) >= ? OR agent_gone_at IS NOT NULL) LIMIT 1"
        );
        let mut args: Vec<Value> = fam.into_iter().map(Value::Text).collect();
        args.push(Value::Integer(MAX_HOOK_MISSES));
        let mut stmt = db.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
        Ok(rows.next()?.is_some())
    };
    bounded(|| read_only(path), look, PENDING_TIMEOUT).unwrap_or(false)
}

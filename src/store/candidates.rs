//! The answers a session's hooks may carry: the fast read-only check (Python
//! `hook._has_candidates`) and the rows themselves (`Store.hook_candidates`).
//! Both use the same filter.

use std::path::Path;

use rusqlite::types::Value;

use super::busy::{PENDING_TIMEOUT, bounded};
use super::connect::read_only;
use super::family::{Continuations, family};
use super::question::{COLUMNS, VIEW};
use super::{MAX_HOOK_MISSES, Question, Store};

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

impl Store {
    /// The answers of `session_id`'s family (see [`family`]) not yet
    /// delivered or seen that a hook may still carry, by id.
    pub fn hook_candidates(
        &self,
        session_id: &str,
        max_misses: i64,
        files: &impl Continuations,
    ) -> rusqlite::Result<Vec<Question>> {
        let fam = family(self.db(), session_id, files)?;
        let marks = vec!["?"; fam.len()].join(",");
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE session_id IN ({marks}) AND status = 'answered' \
             AND delivered_at IS NULL AND acknowledged_at IS NULL AND id NOT IN (SELECT id FROM question_ext \
             WHERE COALESCE(hook_misses, 0) >= ? OR agent_gone_at IS NOT NULL) ORDER BY id"
        );
        let mut args: Vec<Value> = fam.into_iter().map(Value::Text).collect();
        args.push(Value::Integer(max_misses));
        let mut stmt = self.db().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), Question::from_row)?;
        rows.collect()
    }
}

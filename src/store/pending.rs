//! A resumed or compacted session's questions still with the operator (Python
//! `hook._pending`), listed after the rules at SessionStart.

use std::path::Path;

use super::busy::{PENDING_TIMEOUT, bounded};
use super::connect::read_only;
use crate::text::squash;

/// One line per open or answered-but-unseen question of `session_id`, by id:
/// `Q7 (open): …` or `Q7 (answered, not yet seen): … -> …`. Read-only and
/// bounded by [`PENDING_TIMEOUT`]; empty when the database is missing,
/// locked or unreadable.
pub fn pending_lines(path: &Path, session_id: &str) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }
    let rows = bounded(
        || read_only(path),
        |db| {
            let mut stmt = db.prepare(
                "SELECT id, status, question, answer FROM questions WHERE session_id = ? \
                 AND (status = 'open' OR (status = 'answered' AND acknowledged_at IS NULL)) ORDER BY id",
            )?;
            stmt.query_map([session_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        },
        PENDING_TIMEOUT,
    );
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.into_iter()
        .map(|(id, status, question, answer)| {
            let q = squash(&question);
            if status.as_deref() == Some("open") {
                format!("Q{id} (open): {q}")
            } else {
                // Python's f-string prints a NULL answer as `None`
                let answer = answer.as_deref().unwrap_or("None");
                format!("Q{id} (answered, not yet seen): {q} -> {answer}")
            }
        })
        .collect()
}

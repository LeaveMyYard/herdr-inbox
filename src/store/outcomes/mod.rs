//! The delivery state of answers (Python `Store`): what is pending, failed,
//! gone, hook-sent and verified, and the per-answer counters the delivery
//! policy reads. The SQL is Python's, statement for statement; a write that
//! Python runs in `with self.transaction()` runs in [`Store::immediate`] here.

mod gone;
mod hook;
mod live;
mod piles;
mod retry;

use std::time::Duration;

use rusqlite::types::FromSql;
use rusqlite::{OptionalExtension, params};

use super::Store;
use super::question::COLUMNS;

/// The `delivery_error` prefix of an answer whose agent is gone (Python `AGENT_GONE`).
pub const AGENT_GONE: &str = "agent gone: ";
/// An answer never attempted for this long counts as undelivered (its process
/// died before the first try; Python `NEVER_ATTEMPTED_GRACE`).
pub const NEVER_ATTEMPTED_GRACE: Duration = Duration::from_secs(60);

/// [`COLUMNS`] of the view under the alias `v` (for joins with `question_ext`).
fn view_columns() -> String {
    COLUMNS
        .split(',')
        .map(|c| format!("v.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Store {
    /// One `question_ext` column of `qid`: `None` without a row (or NULL).
    fn ext<T: FromSql>(&self, column: &str, qid: i64) -> rusqlite::Result<Option<T>> {
        Ok(self
            .db()
            .query_row(
                &format!("SELECT {column} FROM question_ext WHERE id = ?"),
                params![qid],
                |r| r.get::<_, Option<T>>(0),
            )
            .optional()?
            .flatten())
    }
}

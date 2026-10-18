//! A session's continuation and the liveness of its hooks and watcher
//! (Python `continued_in`, `record_continuation`, `watch_live`, `hook_live`).

use std::time::Duration;

use rusqlite::{OptionalExtension, params};

use crate::clock::now_minus;
use crate::store::Store;

impl Store {
    fn session_column(&self, column: &str, session_id: &str) -> rusqlite::Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                &format!("SELECT {column} FROM sessions WHERE session_id = ?"),
                params![session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// The session `session_id`'s conversation continued in, as recorded.
    pub fn continued_in(&self, session_id: Option<&str>) -> rusqlite::Result<Option<String>> {
        match session_id.filter(|s| !s.is_empty()) {
            Some(s) => self.session_column("continued_in", s),
            None => Ok(None),
        }
    }

    /// Claude Code continued `old`'s conversation in `new` (rows keep the
    /// session they were filed under; ownership goes through this record).
    pub fn record_continuation(&self, old: &str, new: &str) -> rusqlite::Result<()> {
        if old.is_empty()
            || new.is_empty()
            || old == new
            || self.continued_in(Some(old))?.as_deref() == Some(new)
        {
            return Ok(());
        }
        self.db().execute(
            "INSERT INTO sessions (session_id, continued_in) VALUES (?, ?) \
             ON CONFLICT(session_id) DO UPDATE SET continued_in = excluded.continued_in",
            params![old, new],
        )?;
        Ok(())
    }

    /// The session's rewake watcher stamped `watch_seen_at` within `within`.
    pub fn watch_live(&self, session_id: Option<&str>, within: Duration) -> rusqlite::Result<bool> {
        self.seen_within("watch_seen_at", session_id, within)
    }

    /// The session's delivery hook fired (`hook_seen_at`) within `within`.
    pub fn hook_live(&self, session_id: Option<&str>, within: Duration) -> rusqlite::Result<bool> {
        self.seen_within("hook_seen_at", session_id, within)
    }

    fn seen_within(
        &self,
        column: &str,
        session_id: Option<&str>,
        within: Duration,
    ) -> rusqlite::Result<bool> {
        let Some(s) = session_id.filter(|s| !s.is_empty()) else {
            return Ok(false);
        };
        let at = self.session_column(column, s)?.filter(|a| !a.is_empty());
        Ok(at.is_some_and(|at| at >= now_minus(within)))
    }
}

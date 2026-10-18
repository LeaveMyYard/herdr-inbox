//! Verifying hook sends (Python `hook_unverified`, `hook_verified`,
//! `hook_unseen`).

use rusqlite::params;

use super::view_columns;
use crate::clock::now;
use crate::store::Store;
use crate::store::question::{Question, VIEW};
use crate::store::questions::PENDING;

impl Store {
    /// Hook-sent, not yet verified answers of herdr server `server` (or of
    /// none recorded), each with its `hook_sent_at`.
    pub fn hook_unverified(&self, server: &str) -> rusqlite::Result<Vec<(Question, String)>> {
        let sql = format!(
            "SELECT {}, e.hook_sent_at FROM {VIEW} v JOIN question_ext e ON e.id = v.id \
             WHERE e.hook_sent_at IS NOT NULL AND e.verified_at IS NULL AND v.delivered_at IS NOT NULL \
             AND COALESCE(e.herdr_server, ?) = ? ORDER BY v.id",
            view_columns()
        );
        let mut stmt = self.db().prepare(&sql)?;
        let rows = stmt.query_map(params![server, server], |r| {
            Ok((Question::from_row(r)?, r.get(29)?))
        })?;
        rows.collect()
    }

    /// The hook send of `qid` is in the conversation: verified (and seen,
    /// with `acknowledge`).
    pub fn hook_verified(&self, qid: i64, acknowledge: bool) -> rusqlite::Result<()> {
        self.immediate(|db| {
            if acknowledge {
                db.execute(
                    "UPDATE questions SET acknowledged_at = COALESCE(acknowledged_at, ?) WHERE id = ?",
                    params![now(), qid],
                )?;
            }
            db.execute("UPDATE question_ext SET verified_at = ? WHERE id = ?", params![now(), qid])?;
            Ok(())
        })
    }

    /// The send at `sent_at` did not show up in time: back to pending
    /// `reason` with a miss counted; with `reason` `None` (no transcript to
    /// look in) it stays sent-unverified for check_answers. Only for that very
    /// send (compare-and-set on `sent_at`); `false` when a newer one replaced it.
    pub fn hook_unseen(
        &self,
        qid: i64,
        sent_at: &str,
        reason: Option<&str>,
    ) -> rusqlite::Result<bool> {
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE question_ext SET hook_sent_at = NULL, hook_misses = COALESCE(hook_misses, 0) + ? \
                 WHERE id = ? AND hook_sent_at = ?",
                params![i64::from(reason.is_some()), qid, sent_at],
            )?;
            if n != 1 {
                return Ok(false);
            }
            if let Some(reason) = reason {
                db.execute(
                    "UPDATE questions SET delivered_at = NULL, delivery_error = ? WHERE id = ? AND acknowledged_at IS NULL",
                    params![format!("{PENDING}{reason}"), qid],
                )?;
            }
            Ok(true)
        })
    }
}

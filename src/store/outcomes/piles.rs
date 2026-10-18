//! The answers the delivery worker and the popup act on (Python
//! `failed_deliveries`, `undelivered`, `undelivered_all`, `stuck_answers`,
//! `failed_pile`, `clear_seen_errors`).

use std::time::Duration;

use rusqlite::types::Value;

use super::NEVER_ATTEMPTED_GRACE;
use crate::clock::now_minus;
use crate::store::Store;
use crate::store::claims::CLAIM_STALE;
use crate::store::question::{COLUMNS, Question, VIEW};
use crate::store::questions::PENDING;

impl Store {
    /// Answers whose delivery failed and that the agent has not fetched,
    /// unless hidden. A claim in flight is no failure; a stale one is, and so
    /// is an answer never attempted for [`NEVER_ATTEMPTED_GRACE`].
    pub fn failed_deliveries(&self) -> rusqlite::Result<Vec<Question>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE status = 'answered' AND delivered_at IS NULL \
             AND ((delivery_error IS NOT NULL AND (delivery_error NOT LIKE 'sending:%' OR delivery_error < ?)) \
                  OR (delivery_error IS NULL AND answered_at < ?)) \
             AND acknowledged_at IS NULL AND hidden_at IS NULL ORDER BY id"
        );
        let stale = format!("sending:{}", now_minus(CLAIM_STALE));
        self.questions(
            &sql,
            vec![stale.into(), now_minus(NEVER_ATTEMPTED_GRACE).into()],
        )
    }

    /// Answers for `pane_id` to retry: failed or never attempted, not seen,
    /// agent not gone. An error starting with a `caps` prefix is given up
    /// after that many attempts; other errors are retried indefinitely.
    pub fn undelivered(
        &self,
        pane_id: &str,
        caps: &[(&str, i64)],
    ) -> rusqlite::Result<Vec<Question>> {
        let clauses = if caps.is_empty() {
            "0".to_string()
        } else {
            vec!["(delivery_error LIKE ? AND delivery_attempts >= ?)"; caps.len()].join(" OR ")
        };
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE pane_id = ? AND status = 'answered' AND delivered_at IS NULL \
             AND acknowledged_at IS NULL AND (delivery_error IS NOT NULL OR answered_at < ?) \
             AND id NOT IN (SELECT id FROM question_ext WHERE agent_gone_at IS NOT NULL) \
             AND NOT ({clauses}) ORDER BY id"
        );
        let mut args: Vec<Value> = vec![
            pane_id.to_string().into(),
            now_minus(NEVER_ATTEMPTED_GRACE).into(),
        ];
        for (prefix, cap) in caps {
            args.push(format!("{prefix}%").into());
            args.push((*cap).into());
        }
        self.questions(&sql, args)
    }

    /// [`Store::undelivered`] for every pane with an unseen undelivered answer.
    pub fn undelivered_all(&self, caps: &[(&str, i64)]) -> rusqlite::Result<Vec<Question>> {
        let panes: Vec<String> = {
            let mut stmt = self.db().prepare(&format!(
                "SELECT DISTINCT pane_id FROM {VIEW} WHERE pane_id IS NOT NULL AND status = 'answered' \
                 AND delivered_at IS NULL AND acknowledged_at IS NULL"
            ))?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::new();
        for pane in panes {
            out.extend(self.undelivered(&pane, caps)?);
        }
        Ok(out)
    }

    /// Answered and not seen, never delivered or delivered without a
    /// transcript to verify in: candidates for the "not delivered" notice.
    pub fn stuck_answers(&self) -> rusqlite::Result<Vec<Question>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE status = 'answered' AND acknowledged_at IS NULL \
             AND verified_at IS NULL AND pane_id IS NOT NULL \
             AND NOT (delivered_at IS NOT NULL AND delivery_error IS NULL) ORDER BY id"
        );
        self.questions(&sql, Vec::new())
    }

    /// Unseen answers answered within `within` and shown as failed (not
    /// pending, no claim in flight): reconcile candidates.
    pub fn failed_pile(&self, within: Duration) -> rusqlite::Result<Vec<Question>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE status = 'answered' AND acknowledged_at IS NULL AND delivered_at IS NULL \
             AND answered_at >= ? AND delivery_error IS NOT NULL AND delivery_error NOT LIKE 'sending:%' \
             AND delivery_error NOT LIKE ? ORDER BY id"
        );
        self.questions(
            &sql,
            vec![now_minus(within).into(), format!("{PENDING}%").into()],
        )
    }

    /// Seen answers that still carry a delivery error lose it; the count.
    pub fn clear_seen_errors(&self) -> rusqlite::Result<usize> {
        self.db().execute(
            "UPDATE questions SET delivery_error = NULL WHERE acknowledged_at IS NOT NULL \
             AND delivery_error IS NOT NULL AND delivery_error NOT LIKE 'sending:%'",
            [],
        )
    }
}

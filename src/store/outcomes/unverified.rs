//! Answers marked seen without a verified delivery (Python
//! `Store._reconcile_unverified`, the store half): which rows to look at,
//! and what to write once their transcripts were read. The transcript side
//! is `worker::duties::unverified`.

use std::time::Duration;

use inbox_core::deliver::REQUEUED;
use rusqlite::{OptionalExtension, params};

use crate::clock::{now, now_minus};
use crate::store::Store;

/// The meta key of the one-off 0.5.1 sweep (Python `RESET_KEY`).
pub const RESET_KEY: &str = "0.5.1 unverified acknowledgements reset";
/// The one-off sweep looks this far back (Python `RESET_WINDOW`).
pub const RESET_WINDOW: Duration = Duration::from_secs(7 * 86_400);
/// A pre-0.5.1 server's signature is looked for this far back (Python `SIGNATURE_WINDOW`).
pub const SIGNATURE_WINDOW: Duration = Duration::from_secs(86_400);
/// Rows per pass (Python `RECONCILE_BATCH`).
pub const RECONCILE_BATCH: i64 = 200;
/// An old server's unlanded delivery this recent is sent again (Python `REQUEUE_WINDOW`).
pub const REQUEUE_WINDOW: Duration = Duration::from_secs(3600);
/// A row is looked at only once its delivery is this old (Python `REQUEUE_SETTLE`).
pub const REQUEUE_SETTLE: Duration = Duration::from_secs(20);
/// A row is reset at most this often (Python `MAX_RESETS`).
pub const MAX_RESETS: i64 = 2;

/// One row to re-check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unverified {
    /// The question.
    pub id: i64,
    /// Its asking session.
    pub session: Option<String>,
    /// An old server's "typed = seen", recent enough to type again.
    pub recent_signature: bool,
}

/// What became of a re-checked row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recheck {
    /// Found in its transcripts: verified.
    Found,
    /// An old server typed it and called it seen; it never landed: requeued.
    Requeue,
    /// Not found: un-seen, so `check_answers` returns it.
    Unsee,
}

impl Store {
    /// Whether the one-off sweep is still to do, and the rows to re-check.
    pub fn unverified_rows(&self) -> rusqlite::Result<(bool, Vec<Unverified>)> {
        let one_off = self
            .db()
            .prepare("SELECT 1 FROM meta WHERE key = ?")?
            .query_row(params![RESET_KEY], |_| Ok(()))
            .optional()?
            .is_none();
        let mut stmt = self.db().prepare(
            "SELECT q.id, q.session_id, (q.acknowledged_at = q.delivered_at AND q.delivered_at >= ?) \
             FROM questions q LEFT JOIN question_ext e ON e.id = q.id \
             WHERE q.delivered_at IS NOT NULL AND q.acknowledged_at IS NOT NULL AND e.verified_at IS NULL \
             AND COALESCE(e.reset_count, 0) < ? AND q.delivered_at <= ? \
             AND ((? AND q.answered_at >= ?) OR (q.acknowledged_at = q.delivered_at AND q.delivered_at >= ?)) \
             ORDER BY q.id LIMIT ?",
        )?;
        let rows = stmt.query_map(
            params![
                now_minus(REQUEUE_WINDOW),
                MAX_RESETS,
                now_minus(REQUEUE_SETTLE),
                i64::from(one_off),
                now_minus(RESET_WINDOW),
                now_minus(SIGNATURE_WINDOW),
                RECONCILE_BATCH
            ],
            |r| {
                Ok(Unverified {
                    id: r.get(0)?,
                    session: r.get(1)?,
                    recent_signature: r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                })
            },
        )?;
        Ok((one_off, rows.collect::<rusqlite::Result<_>>()?))
    }

    /// Writes the re-checks in one transaction; the one-off sweep is done
    /// once a pass saw fewer rows than a batch.
    pub fn apply_unverified(
        &self,
        one_off: bool,
        outcomes: &[(i64, Recheck)],
    ) -> rusqlite::Result<()> {
        self.immediate(|db| {
            for (qid, what) in outcomes {
                match what {
                    Recheck::Found => {
                        db.execute(
                            "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?) \
                             ON CONFLICT(id) DO UPDATE SET verified_at = excluded.verified_at",
                            params![qid, qid, now()],
                        )?;
                    }
                    Recheck::Requeue | Recheck::Unsee => {
                        if *what == Recheck::Requeue {
                            db.execute(
                                "UPDATE questions SET acknowledged_at = NULL, delivered_at = NULL, \
                                 delivery_error = ? WHERE id = ?",
                                params![REQUEUED, qid],
                            )?;
                        } else {
                            db.execute("UPDATE questions SET acknowledged_at = NULL WHERE id = ?", params![qid])?;
                        }
                        db.execute(
                            "INSERT INTO question_ext (id, group_id, reset_count) VALUES (?, ?, 1) \
                             ON CONFLICT(id) DO UPDATE SET reset_count = COALESCE(reset_count, 0) + 1",
                            params![qid, qid],
                        )?;
                    }
                }
            }
            if one_off && (outcomes.len() as i64) < RECONCILE_BATCH {
                db.execute(
                    "INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)",
                    params![RESET_KEY, now()],
                )?;
            }
            Ok(())
        })
    }
}

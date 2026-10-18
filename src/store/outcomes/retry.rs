//! Pending and retry bookkeeping (Python `set_pending`, `record_busy_miss`,
//! `busy_attempts`, `get_pending_since`, `hook_misses`, `agent_gone`,
//! `legacy_check_due`, `legacy_miss`, `set_server`, `server_of`, `notified`,
//! `mark_notified`).

use std::time::Duration;

use rusqlite::params;

use crate::clock::{now, now_minus};
use crate::store::questions::PENDING;
use crate::store::{Claim, Store};

impl Store {
    /// Not sent yet (e.g. the agent is mid-turn): pending `reason`, retried,
    /// not a failed attempt. A `busy:` reason starts `pending_since` (kept
    /// until delivery: the busy grace counts from it).
    pub fn set_pending(&self, qid: i64, claim: &Claim, reason: &str) -> rusqlite::Result<()> {
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE questions SET delivery_error = ? WHERE id = ? AND delivery_error = ?",
                params![format!("{PENDING}{reason}"), qid, claim.0],
            )?;
            if n > 0 && reason.starts_with("busy:") {
                db.execute(
                    "INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?) \
                     ON CONFLICT(id) DO UPDATE SET pending_since = COALESCE(pending_since, excluded.pending_since)",
                    params![qid, qid, now()],
                )?;
            }
            Ok(())
        })
    }

    /// Typed while busy but not seen: back to pending `reason`, counted in
    /// `busy_attempts` (not a failed attempt).
    pub fn record_busy_miss(&self, qid: i64, claim: &Claim, reason: &str) -> rusqlite::Result<()> {
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE questions SET delivery_error = ? WHERE id = ? AND delivery_error = ?",
                params![format!("{PENDING}{reason}"), qid, claim.0],
            )?;
            if n > 0 {
                db.execute(
                    "INSERT INTO question_ext (id, group_id, busy_attempts) VALUES (?, ?, 1) \
                     ON CONFLICT(id) DO UPDATE SET busy_attempts = COALESCE(busy_attempts, 0) + 1",
                    params![qid, qid],
                )?;
            }
            Ok(())
        })
    }

    /// Typed-while-busy misses of `qid`.
    pub fn busy_attempts(&self, qid: i64) -> rusqlite::Result<i64> {
        Ok(self.ext::<i64>("busy_attempts", qid)?.unwrap_or(0))
    }

    /// Since when `qid` waits for a busy agent.
    pub fn pending_since(&self, qid: i64) -> rusqlite::Result<Option<String>> {
        self.ext("pending_since", qid)
    }

    /// Hook sends of `qid` that never showed up in the conversation.
    pub fn hook_misses(&self, qid: i64) -> rusqlite::Result<i64> {
        Ok(self.ext::<i64>("hook_misses", qid)?.unwrap_or(0))
    }

    /// Whether `qid`'s agent was marked gone.
    pub fn agent_gone(&self, qid: i64) -> rusqlite::Result<bool> {
        Ok(self
            .ext::<String>("agent_gone_at", qid)?
            .is_some_and(|s| !s.is_empty()))
    }

    /// Whether `qid`'s legacy check is due (not done within `every`).
    pub fn legacy_check_due(&self, qid: i64, every: Duration) -> rusqlite::Result<bool> {
        let at = self
            .ext::<String>("legacy_checked_at", qid)?
            .filter(|s| !s.is_empty());
        Ok(at.is_none_or(|at| at <= now_minus(every)))
    }

    /// One more "that pane does not hold the asking conversation": the count.
    pub fn legacy_miss(&self, qid: i64) -> rusqlite::Result<i64> {
        self.db().execute(
            "UPDATE question_ext SET legacy_misses = COALESCE(legacy_misses, 0) + 1, legacy_checked_at = ? WHERE id = ?",
            params![now(), qid],
        )?;
        Ok(self.ext::<i64>("legacy_misses", qid)?.unwrap_or(0))
    }

    /// The herdr server (its socket) `qid` was asked on.
    pub fn set_server(&self, qid: i64, server: Option<&str>) -> rusqlite::Result<()> {
        self.db().execute(
            "UPDATE question_ext SET herdr_server = ? WHERE id = ?",
            params![server, qid],
        )?;
        Ok(())
    }

    /// The herdr server `qid` was asked on.
    pub fn server_of(&self, qid: i64) -> rusqlite::Result<Option<String>> {
        self.ext("herdr_server", qid)
    }

    /// Whether the operator was notified that `qid` is stuck.
    pub fn notified(&self, qid: i64) -> rusqlite::Result<bool> {
        Ok(self
            .ext::<String>("notified_at", qid)?
            .is_some_and(|s| !s.is_empty()))
    }

    /// The operator was notified about these answers.
    pub fn mark_notified(&self, qids: &[i64]) -> rusqlite::Result<()> {
        for qid in qids {
            self.db().execute(
                "INSERT INTO question_ext (id, group_id, notified_at) VALUES (?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET notified_at = excluded.notified_at",
                params![qid, qid, now()],
            )?;
        }
        Ok(())
    }
}

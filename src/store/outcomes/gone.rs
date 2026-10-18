//! Agents that went away, panes that moved, answers found after all (Python
//! `mark_agent_gone`, `move_to_pane`, `gone_recent`, `gone_candidates`,
//! `archive_gone`, `mark_seen_elsewhere`, `mark_landed`).

use std::time::Duration;

use rusqlite::params;
use rusqlite::types::Value;

use super::{AGENT_GONE, view_columns};
use crate::clock::{now, now_minus};
use crate::store::question::{COLUMNS, Question, VIEW};
use crate::store::questions::PENDING;
use crate::store::{Claim, Store};

impl Store {
    /// `qid`'s agent is gone: no more retries; the answer stays unseen for
    /// check_answers. A claim in flight keeps its `delivery_error`.
    pub fn mark_agent_gone(&self, qid: i64, reason: &str) -> rusqlite::Result<()> {
        self.immediate(|db| {
            db.execute(
                "UPDATE question_ext SET agent_gone_at = COALESCE(agent_gone_at, ?) WHERE id = ?",
                params![now(), qid],
            )?;
            db.execute(
                "UPDATE questions SET delivery_error = ? WHERE id = ? AND delivered_at IS NULL \
                 AND COALESCE(delivery_error, '') NOT LIKE 'sending:%'",
                params![format!("{AGENT_GONE}{reason}"), qid],
            )?;
            Ok(())
        })
    }

    /// The asking conversation runs in `pane_id` now: deliver there (pending
    /// `reason`, attempts reset, an earlier "agent gone" cleared). Under
    /// `claim` when one is held.
    pub fn move_to_pane(
        &self,
        qid: i64,
        pane_id: &str,
        tab_id: Option<&str>,
        reason: &str,
        claim: Option<&Claim>,
        tab_label: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let mine = if claim.is_some() {
            " AND delivery_error = ?"
        } else {
            ""
        };
        let mut args: Vec<Value> = vec![
            pane_id.to_string().into(),
            tab_id.map(String::from).into(),
            tab_label.map(String::from).into(),
            format!("{PENDING}{reason}").into(),
            qid.into(),
        ];
        if let Some(c) = claim {
            args.push(c.0.clone().into());
        }
        self.immediate(|db| {
            let n = db.execute(
                &format!(
                    "UPDATE questions SET pane_id = ?, tab_id = COALESCE(?, tab_id), tab_label = COALESCE(?, tab_label), \
                     delivery_error = ?, delivery_attempts = 0 \
                     WHERE id = ? AND status = 'answered' AND acknowledged_at IS NULL{mine}"
                ),
                rusqlite::params_from_iter(args),
            )?;
            if n > 0 {
                db.execute("UPDATE question_ext SET agent_gone_at = NULL WHERE id = ?", params![qid])?;
            }
            Ok(n == 1)
        })
    }

    /// Answers marked "agent gone" within `within`, still unseen, not hidden,
    /// no send in flight: maybe their pane only moved.
    pub fn gone_recent(&self, within: Duration) -> rusqlite::Result<Vec<Question>> {
        let sql = format!(
            "SELECT {} FROM {VIEW} v JOIN question_ext e ON e.id = v.id WHERE e.agent_gone_at >= ? \
             AND v.status = 'answered' AND v.acknowledged_at IS NULL AND v.delivered_at IS NULL \
             AND v.session_id IS NOT NULL AND e.hidden_at IS NULL \
             AND COALESCE(v.delivery_error, '') NOT LIKE 'sending:%' ORDER BY v.id",
            view_columns()
        );
        self.questions(&sql, vec![now_minus(within).into()])
    }

    /// Undelivered answers whose error starts with one of `prefixes` and that
    /// ran out of `cap` attempts, not yet marked gone.
    pub fn gone_candidates(&self, prefixes: &[&str], cap: i64) -> rusqlite::Result<Vec<Question>> {
        let clauses = vec!["delivery_error LIKE ?"; prefixes.len()].join(" OR ");
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE status = 'answered' AND delivered_at IS NULL AND acknowledged_at IS NULL \
             AND ({clauses}) AND delivery_attempts >= ? \
             AND id NOT IN (SELECT id FROM question_ext WHERE agent_gone_at IS NOT NULL) ORDER BY id"
        );
        let mut args: Vec<Value> = prefixes.iter().map(|p| format!("{p}%").into()).collect();
        args.push(cap.into());
        self.questions(&sql, args)
    }

    /// Answers whose agent is gone for longer than `after` leave the popup
    /// (hidden); the count.
    pub fn archive_gone(&self, after: Duration) -> rusqlite::Result<usize> {
        self.db().execute(
            "UPDATE question_ext SET hidden_at = ? WHERE agent_gone_at IS NOT NULL AND agent_gone_at <= ? \
             AND hidden_at IS NULL AND id IN (SELECT id FROM questions WHERE status = 'answered' \
             AND acknowledged_at IS NULL)",
            params![now(), now_minus(after)],
        )
    }

    /// The agent has `qid` some other way: seen, never typed again, not a
    /// failure (not "delivered"). Under `claim`, or with no claim in flight.
    pub fn mark_seen_elsewhere(&self, qid: i64, claim: Option<&Claim>) -> rusqlite::Result<bool> {
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE questions SET acknowledged_at = COALESCE(acknowledged_at, ?), delivery_error = NULL \
                 WHERE id = ? AND (delivery_error IS ? OR COALESCE(delivery_error, '') NOT LIKE 'sending:%')",
                params![now(), qid, claim.map(|c| c.0.as_str())],
            )?;
            if n > 0 {
                db.execute("UPDATE question_ext SET agent_gone_at = NULL WHERE id = ?", params![qid])?;
            }
            Ok(n == 1)
        })
    }

    /// Found in a transcript after all: delivered and verified (seen too
    /// with `acknowledge`); the error and any "agent gone" cleared.
    pub fn mark_landed(&self, qid: i64, acknowledge: bool) -> rusqlite::Result<bool> {
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE questions SET delivered_at = COALESCE(delivered_at, ?), delivery_error = NULL, \
                 acknowledged_at = CASE WHEN ? THEN COALESCE(acknowledged_at, ?) ELSE acknowledged_at END \
                 WHERE id = ? AND COALESCE(delivery_error, '') NOT LIKE 'sending:%'",
                params![now(), i64::from(acknowledge), now(), qid],
            )?;
            if n > 0 {
                db.execute(
                    "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?) \
                     ON CONFLICT(id) DO UPDATE SET verified_at = COALESCE(verified_at, excluded.verified_at), \
                     agent_gone_at = NULL",
                    params![qid, qid, now()],
                )?;
            }
            Ok(n == 1)
        })
    }
}

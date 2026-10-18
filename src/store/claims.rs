//! The delivery claim and the outcomes written under it (Python
//! `Store.claim_delivery`, `release_claim`, `record_delivery`,
//! `record_hook_delivery`), byte for byte: Python and Rust processes claim
//! the same rows during the migration (plan §2.3, §5.4).
//!
//! A claim is a compare-and-set of `delivery_error` to
//! `sending:<at>:<token>`: only one process handles an answer at a time, and
//! one whose claim is older than [`CLAIM_STALE`] died mid-send and can be
//! taken over. Every outcome is written only while the claim is still ours.

use std::time::Duration;

use rusqlite::params;

use super::Store;
use crate::clock::{now, now_minus};
use crate::token::hex8;

/// A claim older than this belongs to a process that died mid-send.
pub const CLAIM_STALE: Duration = Duration::from_secs(60);

/// A claim on one answer: the exact `delivery_error` value written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim(pub String);

impl Store {
    /// Claims answer `qid` for delivery: answered, not delivered, not fetched
    /// by its agent, and not claimed by a live process. `None` when another
    /// process holds it (or it is not deliverable).
    pub fn claim_delivery(&self, qid: i64) -> rusqlite::Result<Option<Claim>> {
        let claim = format!("sending:{}:{}", now(), hex8());
        let stale = format!("sending:{}", now_minus(CLAIM_STALE));
        let n = self.db().execute(
            "UPDATE questions SET delivery_error = ? WHERE id = ? AND status = 'answered' AND delivered_at IS NULL \
             AND acknowledged_at IS NULL \
             AND (delivery_error IS NULL OR delivery_error NOT LIKE 'sending:%' OR delivery_error < ?)",
            params![claim, qid, stale],
        )?;
        Ok((n == 1).then_some(Claim(claim)))
    }

    /// Gives a claim back untouched: `delivery_error` returns to `previous`.
    pub fn release_claim(
        &self,
        qid: i64,
        claim: &Claim,
        previous: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.db().execute(
            "UPDATE questions SET delivery_error = ? WHERE id = ? AND delivery_error = ?",
            params![previous, qid, claim.0],
        )?;
        Ok(())
    }

    /// Under `claim`: handed to Claude Code as hook context (sent, not yet
    /// seen; the worker verifies it). `false` when the claim was lost.
    pub fn record_hook_delivery(&self, qid: i64, claim: &Claim) -> rusqlite::Result<bool> {
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE questions SET delivered_at = ?, delivery_error = NULL WHERE id = ? AND delivery_error = ?",
                params![now(), qid, claim.0],
            )?;
            if n != 1 {
                return Ok(false);
            }
            db.execute(
                "INSERT INTO question_ext (id, group_id, hook_sent_at) VALUES (?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET hook_sent_at = excluded.hook_sent_at, pending_since = NULL",
                params![qid, qid, now()],
            )?;
            Ok(true)
        })
    }

    /// Writes a delivery outcome; with `claim`, only while that claim is
    /// still ours. `error`: a failed attempt (counted). Otherwise delivered:
    /// only a `verified` delivery (found in the transcript) sets
    /// `acknowledged_at` (with `acknowledge`) and `verified_at`, in one
    /// transaction with `delivered_at`.
    pub fn record_delivery(
        &self,
        qid: i64,
        error: Option<&str>,
        acknowledge: bool,
        claim: Option<&Claim>,
        verified: bool,
    ) -> rusqlite::Result<()> {
        let (mine, arg) = match claim {
            Some(c) => (" AND delivery_error = ?", Some(c.0.as_str())),
            None => ("", None),
        };
        if let Some(error) = error.filter(|e| !e.is_empty()) {
            let sql = format!(
                "UPDATE questions SET delivery_error = ?, delivery_attempts = delivery_attempts + 1 WHERE id = ?{mine}"
            );
            match arg {
                Some(c) => self.db().execute(&sql, params![error, qid, c])?,
                None => self.db().execute(&sql, params![error, qid])?,
            };
            return Ok(());
        }
        let ack = (acknowledge && verified).then(now);
        self.immediate(|db| {
            let sql = format!(
                "UPDATE questions SET delivered_at = ?, acknowledged_at = COALESCE(acknowledged_at, ?), \
                 delivery_error = NULL WHERE id = ?{mine}"
            );
            let n = match arg {
                Some(c) => db.execute(&sql, params![now(), ack, qid, c])?,
                None => db.execute(&sql, params![now(), ack, qid])?,
            };
            if n > 0 {
                db.execute("UPDATE question_ext SET pending_since = NULL WHERE id = ?", [qid])?;
            }
            if verified && n > 0 {
                db.execute(
                    "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?) \
                     ON CONFLICT(id) DO UPDATE SET verified_at = excluded.verified_at",
                    params![qid, qid, now()],
                )?;
            }
            Ok(())
        })
    }
}

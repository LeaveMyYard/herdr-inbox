//! Sending the outbox (Python `remote.flush`, `_sending`,
//! `_resolve_expired`, `expire_sweep`): FIFO per question, re-validated
//! against the mirror first, expired ones failed or asked about, each send
//! idempotent by its request id on the remote.

use std::time::{Duration, SystemTime};

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::outbox::{OUTBOX_COLUMNS, OutboxRow};
use super::{CallResult, MACHINE_COLUMNS, Machine, Notice, iso};
use crate::pyjson::{self, dumps};
use crate::remote::command::{FixedCommand, InboxSub, SafeTarget, Ssh};
use crate::remote::fields::text;
use crate::remote::pyvalue::py_repr_str;
use crate::remote::validate::STATUSES;
use crate::store::Store;

/// What a flush did: its notices, and whether it stopped early (Python
/// raised out of `flush`: an unsafe target in a status query, a reply that is
/// JSON but not an object). Rows sent before that stay as they were left.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flushed {
    /// Notices to send.
    pub notices: Vec<Notice>,
    /// Why it stopped early.
    pub aborted: Option<String>,
}

pub(super) fn reply_of(stdout: &str) -> Option<Value> {
    pyjson::loads(stdout).ok()
}

impl Store {
    /// Runs `ssh` with the row marked as being sent meanwhile (Python
    /// `_sending`): the sweep and the popup leave it alone.
    pub(super) fn sending(
        &self,
        request_id: &str,
        now: SystemTime,
        send: impl FnOnce() -> CallResult,
    ) -> rusqlite::Result<CallResult> {
        self.db().execute(
            "UPDATE remote_outbox SET sending_at = ? WHERE request_id = ?",
            params![iso(now), request_id],
        )?;
        let result = send();
        self.db().execute(
            "UPDATE remote_outbox SET sending_at = NULL WHERE request_id = ?",
            [request_id],
        )?;
        Ok(result)
    }

    /// Sends the queued actions of `machine_id` (Python `flush`).
    pub fn flush(
        &self,
        machine_id: &str,
        now: SystemTime,
        ssh: Ssh<'_>,
    ) -> rusqlite::Result<Flushed> {
        let mut out = Flushed::default();
        let rec = self
            .db()
            .query_row(
                &format!("SELECT {MACHINE_COLUMNS} FROM remote_machines WHERE machine_id = ?"),
                [machine_id],
                Machine::from_row,
            )
            .optional()?;
        let Some(rec) = rec.filter(|r| r.last_ok_at.as_deref().is_some_and(|s| !s.is_empty()))
        else {
            return Ok(out);
        };
        let label = rec.name().to_string();
        let queued: Vec<OutboxRow> = {
            let mut stmt = self.db().prepare(&format!(
                "SELECT {OUTBOX_COLUMNS}, NULL FROM remote_outbox o WHERE o.machine_id = ? AND o.state = 'queued' \
                 ORDER BY o.created_at, o.rowid"
            ))?;
            stmt.query_map([machine_id], OutboxRow::from_row)?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut heads: Vec<OutboxRow> = Vec::new();
        for row in queued {
            // FIFO per question: only its oldest queued action goes now (alert ids are their own namespace)
            if !heads.iter().any(|h| {
                (h.action == "ack_alert") == (row.action == "ack_alert")
                    && h.remote_id == row.remote_id
            }) {
                heads.push(row);
            }
        }
        for row in heads {
            if row
                .next_try_at
                .as_deref()
                .is_some_and(|n| n > iso(now).as_str())
            {
                continue;
            }
            if row.expires_at.as_str() <= iso(now).as_str() {
                if row.attempts == 0 {
                    out.notices.extend(self.expire(&row, &label, now)?);
                } else if let Err(e) =
                    self.resolve_expired(&rec, &row, &label, now, ssh, &mut out.notices)?
                {
                    out.aborted = Some(e);
                    return Ok(out);
                }
                continue;
            }
            out.notices
                .extend(self.send_one(&rec, &row, &label, now, ssh)?);
        }
        Ok(out)
    }

    /// One head of the queue: re-validated against the mirror, then sent.
    fn send_one(
        &self,
        rec: &Machine,
        row: &OutboxRow,
        label: &str,
        now: SystemTime,
        ssh: Ssh<'_>,
    ) -> rusqlite::Result<Option<Notice>> {
        let reference = row.reference(label);
        let not_sent = format!("{reference}: {} not sent", row.action);
        let mirror: Option<(String, String, String)> = if row.action == "ack_alert" {
            self.db()
                .query_row(
                    "SELECT db_id, akey, 'any' FROM remote_alerts WHERE machine_id = ? AND alert_id = ?",
                    params![rec.machine_id, row.remote_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?
        } else {
            self.db()
                .query_row(
                    "SELECT db_id, qkey, status FROM remote_questions WHERE machine_id = ? AND remote_id = ?",
                    params![rec.machine_id, row.remote_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?
        };
        let wanted: &[&str] = match row.action.as_str() {
            "answer" | "dismiss" => &["open"],
            "deliver" => &["answered"],
            "hide" => &STATUSES,
            _ => &["any"],
        };
        let fits = mirror.as_ref().is_some_and(|(d, k, s)| {
            *d == row.db_id && *k == row.qkey && wanted.contains(&s.as_str())
        });
        if !fits {
            let reason = match &mirror {
                Some((_, k, s)) if *k == row.qkey => s.clone(),
                _ => "not that question any more".to_string(),
            };
            let body = format!("rejected: {reason}");
            return Ok(self
                .settle(
                    &row.request_id,
                    "rejected",
                    &reason,
                    Some((not_sent, body)),
                    now,
                )?
                .1);
        }
        let detail = row
            .detail_json
            .as_deref()
            .filter(|d| !d.is_empty())
            .and_then(|d| pyjson::loads(d).ok())
            .unwrap_or(Value::Null);
        let request = json!({"request_id": row.request_id, "db_id": row.db_id, "id": row.remote_id, "qkey": row.qkey,
                             "action": row.action, "detail": detail});
        // Python call(): the target, then the cached paths, before ssh runs
        let checked = SafeTarget::for_call(rec.target.as_deref())
            .and_then(|t| Ok((t, FixedCommand::inbox(rec, InboxSub::Apply)?)));
        let (target, command) = match checked {
            Ok(c) => c,
            Err(e) => {
                return Ok(self
                    .settle(
                        &row.request_id,
                        "failed",
                        &e,
                        Some((not_sent, e.clone())),
                        now,
                    )?
                    .1);
            }
        };
        let result = self.sending(&row.request_id, now, || {
            ssh(&target, &command, Some(&dumps(&request)))
        })?;
        if result.code == 2 {
            let msg = format!("upgrade {label} (inbox remote upgrade {label})");
            return Ok(self
                .settle(
                    &row.request_id,
                    "failed",
                    &msg,
                    Some((not_sent, msg.clone())),
                    now,
                )?
                .1);
        }
        let reply = reply_of(&result.stdout);
        let outcome = reply
            .as_ref()
            .and_then(|r| r.as_object())
            .and_then(|r| r.get("outcome"));
        match outcome.and_then(Value::as_str) {
            Some(o @ ("applied" | "already_applied")) => {
                Ok(self.settle(&row.request_id, "done", o, None, now)?.1)
            }
            Some("rejected") => {
                let reason = reason_of(reply.as_ref());
                let title = format!("{reference}: {label} rejected your {}", row.action);
                Ok(self
                    .settle(
                        &row.request_id,
                        "rejected",
                        &reason,
                        Some((title, reason.clone())),
                        now,
                    )?
                    .1)
            }
            _ => {
                // a transport error or a lost reply: retry with the SAME request id
                let delay = (5u64 << row.attempts.clamp(0, 4)).min(60);
                let tail: Vec<char> = result.stderr.chars().collect();
                let tail: String = tail[tail.len().saturating_sub(200)..].iter().collect();
                self.db().execute(
                    "UPDATE remote_outbox SET attempts = attempts + 1, next_try_at = ?, result = ? WHERE request_id = ?",
                    params![iso(now + Duration::from_secs(delay)), tail, row.request_id],
                )?;
                Ok(None)
            }
        }
    }
}

/// `_text(str(reply.get("reason") or "rejected"), 200)`.
pub(super) fn reason_of(reply: Option<&Value>) -> String {
    let raw = reply
        .and_then(|r| r.get("reason"))
        .filter(|v| crate::remote::pyvalue::py_truthy(v));
    let s = raw.map_or_else(|| "rejected".to_string(), py_repr_str);
    text(Some(&Value::String(s)), 200)
        .ok()
        .flatten()
        .unwrap_or_default()
}

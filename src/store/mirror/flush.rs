//! Sending the outbox (Python `remote.flush`, `_sending`,
//! `_resolve_expired`, `expire_sweep`): FIFO per question, re-validated
//! against the mirror first, expired ones failed or asked about, each send
//! idempotent by its request id on the remote.

use std::time::{Duration, SystemTime};

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::outbox::{OUTBOX_COLUMNS, OutboxRow};
use super::paths::too_large;
use super::{Clock, MACHINE_COLUMNS, Machine, Notice, iso};
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

/// The start of a flush's abort text for a corrupt `detail_json` (Python's
/// `json.loads` raising out of `flush`; serde's message follows).
pub const CORRUPT_DETAIL: &str = "corrupt detail_json: ";

/// An `inbox apply` reply: `None` when it is not JSON, or longer than the
/// export cap (Python reads any length; a remote's output is untrusted, so
/// an oversized reply counts as lost and the send is retried with the same
/// request id).
pub(super) fn reply_of(stdout: &str) -> Option<Value> {
    if too_large(stdout) {
        return None;
    }
    pyjson::loads(stdout).ok()
}

impl Store {
    /// Runs `ssh` with the row marked as being sent meanwhile (Python
    /// `_sending`): the sweep and the popup leave it alone.
    ///
    /// `stamp`: the time read as Python reads it, before the call starts.
    pub(super) fn sending<T>(
        &self,
        request_id: &str,
        stamp: SystemTime,
        send: impl FnOnce() -> T,
    ) -> rusqlite::Result<T> {
        self.db().execute(
            "UPDATE remote_outbox SET sending_at = ? WHERE request_id = ?",
            params![iso(stamp), request_id],
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
        clock: Clock<'_>,
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
            // Python reads the time per check, and only when the row has a next try
            if row
                .next_try_at
                .as_deref()
                .is_some_and(|n| !n.is_empty() && n > iso(clock()).as_str())
            {
                continue;
            }
            if row.expires_at.as_str() <= iso(clock()).as_str() {
                if row.attempts == 0 {
                    out.notices.extend(self.expire(&row, &label, clock())?);
                } else if let Err(e) =
                    self.resolve_expired(&rec, &row, &label, clock, ssh, &mut out.notices)?
                {
                    out.aborted = Some(e);
                    return Ok(out);
                }
                continue;
            }
            match self.send_one(&rec, &row, &label, clock, ssh)? {
                Ok(notice) => out.notices.extend(notice),
                Err(e) => {
                    out.aborted = Some(e);
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    /// One head of the queue: re-validated against the mirror, then sent.
    fn send_one(
        &self,
        rec: &Machine,
        row: &OutboxRow,
        label: &str,
        clock: Clock<'_>,
        ssh: Ssh<'_>,
    ) -> rusqlite::Result<Result<Option<Notice>, String>> {
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
            return Ok(Ok(self
                .settle(
                    &row.request_id,
                    "rejected",
                    &reason,
                    Some((not_sent, body)),
                    clock(),
                )?
                .1));
        }
        // Python: json.loads(detail_json) if detail_json else None, outside any try: a corrupt
        // one raises out of flush before anything is sent (only corruption can produce one)
        let detail = row.detail_json.as_deref().filter(|d| !d.is_empty());
        if let Some(Err(e)) = detail.map(pyjson::loads) {
            return Ok(Err(format!("{CORRUPT_DETAIL}{e}")));
        }
        let request = request_text(row, detail);
        // Python call() inside _sending: the stamp, then the target and the cached paths
        let stamp = clock();
        let sent = self.sending(&row.request_id, stamp, || {
            let target = SafeTarget::for_call(rec.target.as_deref())?;
            let command = FixedCommand::inbox(rec, InboxSub::Apply)?;
            Ok::<_, String>(ssh(&target, &command, Some(&request)))
        })?;
        let result = match sent {
            Ok(r) => r,
            Err(e) => {
                return Ok(Ok(self
                    .settle(
                        &row.request_id,
                        "failed",
                        &e,
                        Some((not_sent, e.clone())),
                        clock(),
                    )?
                    .1));
            }
        };
        if result.code == 2 {
            let msg = format!("upgrade {label} (inbox remote upgrade {label})");
            return Ok(Ok(self
                .settle(
                    &row.request_id,
                    "failed",
                    &msg,
                    Some((not_sent, msg.clone())),
                    clock(),
                )?
                .1));
        }
        let reply = reply_of(&result.stdout);
        let outcome = reply
            .as_ref()
            .and_then(|r| r.as_object())
            .and_then(|r| r.get("outcome"));
        match outcome.and_then(Value::as_str) {
            Some(o @ ("applied" | "already_applied")) => Ok(Ok(self
                .settle(&row.request_id, "done", o, None, clock())?
                .1)),
            Some("rejected") => {
                let reason = reason_of(reply.as_ref());
                let title = format!("{reference}: {label} rejected your {}", row.action);
                Ok(Ok(self
                    .settle(
                        &row.request_id,
                        "rejected",
                        &reason,
                        Some((title, reason.clone())),
                        clock(),
                    )?
                    .1))
            }
            _ => {
                // a transport error or a lost reply: retry with the SAME request id
                let delay = (5u64 << row.attempts.clamp(0, 4)).min(60);
                let tail: Vec<char> = result.stderr.chars().collect();
                let tail: String = tail[tail.len().saturating_sub(200)..].iter().collect();
                self.db().execute(
                    "UPDATE remote_outbox SET attempts = attempts + 1, next_try_at = ?, result = ? WHERE request_id = ?",
                    params![iso(clock() + Duration::from_secs(delay)), tail, row.request_id],
                )?;
                Ok(Ok(None))
            }
        }
    }
}

/// The `inbox apply` request, as Python's `json.dumps` writes it, with the
/// stored `detail_json` text itself as its `detail`: Python re-dumps what it
/// loaded, which gives back the stored text for everything either side
/// writes, while a value read here can be lossy (an escaped lone surrogate
/// reads as U+FFFD) and must never be sent back (#79).
fn request_text(row: &OutboxRow, detail: Option<&str>) -> String {
    let head = dumps(
        &json!({"request_id": row.request_id, "db_id": row.db_id, "id": row.remote_id,
                             "qkey": row.qkey, "action": row.action, "detail": null}),
    );
    match detail {
        None => head,
        Some(raw) => {
            let open = head.strip_suffix("null}").expect("detail is the last key");
            format!("{open}{raw}}}")
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

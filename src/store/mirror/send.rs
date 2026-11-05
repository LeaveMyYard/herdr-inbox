//! One head of the outbox queue (Python `flush`'s loop body): re-validated
//! against the mirror, sent with its request id, and settled by the reply.

use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::flush::{reason_of, reply_of};
use super::outbox::OutboxRow;
use super::{Clock, Machine, Notice, iso};
use crate::pyjson::{self, dumps};
use crate::remote::command::{FixedCommand, InboxSub, SafeTarget, Ssh};
use crate::remote::validate::STATUSES;
use crate::store::Store;

/// The start of a flush's abort text for a corrupt `detail_json` (Python's
/// `json.loads` raising out of `flush`; serde's message follows).
pub const CORRUPT_DETAIL: &str = "corrupt detail_json: ";

impl Store {
    /// One head of the queue: re-validated against the mirror, then sent.
    pub(super) fn send_one(
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

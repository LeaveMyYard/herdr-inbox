//! Expired outbox rows (Python `remote._resolve_expired`, `expire_sweep`):
//! a send whose reply was lost is asked about, never applied late; the rest
//! fail with a notice.

use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use super::flush::{reason_of, reply_of};
use super::outbox::{OUTBOX_COLUMNS, OutboxRow};
use super::{Clock, Machine, Notice, iso};
use crate::pyjson::dumps;
use crate::remote::command::{FixedCommand, InboxSub, SafeTarget, Ssh};
use crate::store::Store;

impl Store {
    /// After expiry only a read-only status query (Python `_resolve_expired`):
    /// was this request ever applied there? `Err`: Python raised.
    pub(super) fn resolve_expired(
        &self,
        rec: &Machine,
        row: &OutboxRow,
        label: &str,
        clock: Clock<'_>,
        ssh: Ssh<'_>,
        notices: &mut Vec<Notice>,
    ) -> rusqlite::Result<Result<(), String>> {
        let reference = format!("{label}:Q{}", row.remote_id);
        let request = json!({"request_id": row.request_id, "action": "status"});
        // Python call() inside _sending: the stamp, then the target and the cached paths
        let stamp = clock();
        let sent = self.sending(&row.request_id, stamp, || {
            let target = SafeTarget::for_call(rec.target.as_deref())?;
            let command = FixedCommand::inbox(rec, InboxSub::Apply)?;
            Ok::<_, String>(ssh(&target, &command, Some(&dumps(&request))))
        })?;
        let result = match sent {
            Ok(r) => r,
            Err(e) => return Ok(Err(e)), // Python: the ValueError escapes flush
        };
        let reply = reply_of(&result.stdout).unwrap_or_else(|| json!({}));
        let Some(r) = reply.as_object() else {
            return Ok(Err(format!(
                "'{}' object has no attribute 'get'",
                super::paths::py_type(&reply)
            )));
        };
        let state = r.get("state").and_then(Value::as_str);
        let notice = match r.get("outcome").and_then(Value::as_str) {
            Some("recorded") if state == Some("applied") => {
                self.settle(
                    &row.request_id,
                    "done",
                    "applied before it expired",
                    Some((
                        format!("{reference}: your {} reached {label} after all", row.action),
                        "it was applied before the expiry".into(),
                    )),
                    clock(),
                )?
                .1
            }
            Some("recorded") => {
                let reason = reason_of(Some(&reply));
                let title = format!("{reference}: {label} rejected your {}", row.action);
                self.settle(
                    &row.request_id,
                    "rejected",
                    &reason,
                    Some((title, reason.clone())),
                    clock(),
                )?
                .1
            }
            Some("expired") => {
                self.settle(
                    &row.request_id,
                    "failed",
                    "expired",
                    Some((
                        format!("{reference}: your queued {} expired", row.action),
                        format!(
                            "{label} never applied it (not sent); answer again if still needed"
                        ),
                    )),
                    clock(),
                )?
                .1
            }
            Some("rejected") => self.expire(row, label, clock())?,
            _ => None, // no reply: left as is; the sweep or the next tick decides
        };
        notices.extend(notice);
        Ok(Ok(()))
    }

    /// Expires queued actions past their time for every machine (Python
    /// `expire_sweep`); one sent at least once to a reachable machine waits
    /// for flush's status query instead.
    pub fn expire_sweep(&self, clock: Clock<'_>) -> rusqlite::Result<Vec<Notice>> {
        let rows: Vec<OutboxRow> = {
            let mut stmt = self.db().prepare(&format!(
                "SELECT {OUTBOX_COLUMNS}, m.label FROM remote_outbox o LEFT JOIN remote_machines m USING (machine_id) \
                 WHERE o.state = 'queued' AND o.expires_at <= ?"
            ))?;
            stmt.query_map([iso(clock())], OutboxRow::from_row)?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut notices = Vec::new();
        for row in rows {
            if row.in_flight(clock) {
                continue;
            }
            let reachable = row.label.is_some()
                && self
                    .db()
                    .query_row(
                        "SELECT 1 FROM remote_machines WHERE machine_id = ? AND last_error IS NULL AND last_ok_at IS NOT NULL",
                        [&row.machine_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
            if row.attempts != 0 && reachable {
                continue;
            }
            let label = row
                .label
                .clone()
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| row.machine_id.clone());
            notices.extend(self.expire(&row, &label, clock())?);
        }
        Ok(notices)
    }
}

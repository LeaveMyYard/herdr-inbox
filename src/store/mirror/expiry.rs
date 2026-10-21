//! Expired outbox rows (Python `remote._resolve_expired`, `expire_sweep`):
//! a send whose reply was lost is asked about, never applied late; the rest
//! fail with a notice.

use std::time::SystemTime;

use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use super::flush::{reason_of, reply_of};
use super::outbox::{OUTBOX_COLUMNS, OutboxRow};
use super::{Machine, Notice, iso};
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
        now: SystemTime,
        ssh: Ssh<'_>,
        notices: &mut Vec<Notice>,
    ) -> rusqlite::Result<Result<(), String>> {
        let reference = format!("{label}:Q{}", row.remote_id);
        let checked = SafeTarget::for_call(rec.target.as_deref())
            .and_then(|t| Ok((t, FixedCommand::inbox(rec, InboxSub::Apply)?)));
        let (target, command) = match checked {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)), // Python: the ValueError escapes flush
        };
        let request = json!({"request_id": row.request_id, "action": "status"});
        let result = self.sending(&row.request_id, now, || {
            ssh(&target, &command, Some(&dumps(&request)))
        })?;
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
                    now,
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
                    now,
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
                    now,
                )?
                .1
            }
            Some("rejected") => self.expire(row, label, now)?,
            _ => None, // no reply: left as is; the sweep or the next tick decides
        };
        notices.extend(notice);
        Ok(Ok(()))
    }

    /// Expires queued actions past their time for every machine (Python
    /// `expire_sweep`); one sent at least once to a reachable machine waits
    /// for flush's status query instead.
    pub fn expire_sweep(&self, now: SystemTime) -> rusqlite::Result<Vec<Notice>> {
        let rows: Vec<OutboxRow> = {
            let mut stmt = self.db().prepare(&format!(
                "SELECT {OUTBOX_COLUMNS}, m.label FROM remote_outbox o LEFT JOIN remote_machines m USING (machine_id) \
                 WHERE o.state = 'queued' AND o.expires_at <= ?"
            ))?;
            stmt.query_map([iso(now)], OutboxRow::from_row)?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut notices = Vec::new();
        for row in rows {
            if row.in_flight(now) {
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
            notices.extend(self.expire(&row, &label, now)?);
        }
        Ok(notices)
    }
}

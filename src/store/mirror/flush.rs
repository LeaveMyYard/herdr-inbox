//! Sending the outbox (Python `remote.flush`, `_sending`,
//! `_resolve_expired`, `expire_sweep`): FIFO per question, re-validated
//! against the mirror first, expired ones failed or asked about, each send
//! idempotent by its request id on the remote.

use std::time::SystemTime;

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::outbox::{OUTBOX_COLUMNS, OutboxRow};
use super::paths::too_large;
use super::{Clock, MACHINE_COLUMNS, Machine, Notice, iso};
use crate::pyjson;
use crate::remote::command::Ssh;
use crate::remote::fields::text;
use crate::remote::pyvalue::py_repr_str;
use crate::store::{Partial, Store};

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

    /// Sends the queued actions of `machine_id` (Python `flush`). What it
    /// settled before a database error stays in the result: those rows are
    /// never picked up again, so their notices go out all the same.
    pub fn flush(&self, machine_id: &str, clock: Clock<'_>, ssh: Ssh<'_>) -> Partial<Flushed> {
        let mut out = Flushed::default();
        let ended = self.flush_into(machine_id, clock, ssh, &mut out);
        Partial::of(out, ended)
    }

    fn flush_into(
        &self,
        machine_id: &str,
        clock: Clock<'_>,
        ssh: Ssh<'_>,
        out: &mut Flushed,
    ) -> rusqlite::Result<()> {
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
            return Ok(());
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
                    return Ok(());
                }
                continue;
            }
            match self.send_one(&rec, &row, &label, clock, ssh)? {
                Ok(notice) => out.notices.extend(notice),
                Err(e) => {
                    out.aborted = Some(e);
                    return Ok(());
                }
            }
        }
        Ok(())
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

//! The outbox: operator actions on mirrored questions and alerts, queued here
//! and sent by [`Store::flush`] (Python `remote.enqueue`,
//! `enqueue_alert_ack`, `_settle`, `_expire`, `_in_flight`, `clearable`,
//! `outbox_rows`, `mark_seen`).

use std::time::{Duration, SystemTime};

use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Map, Value};

use super::{Notice, iso};
use crate::pyjson::dumps;
use crate::remote::pyvalue::py_truthy;
use crate::remote::validate::py_int;
use crate::store::Store;

/// A send older than this is no longer in flight (Python `SENDING_STALE`:
/// the call timeout plus 20 s).
pub const SENDING_STALE: Duration = Duration::from_secs(30);

/// One row of `remote_outbox` (and the machine's label where joined).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct OutboxRow {
    pub request_id: String,
    pub machine_id: String,
    pub db_id: String,
    pub remote_id: i64,
    pub qkey: String,
    pub action: String,
    pub detail_json: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub state: String,
    pub result: Option<String>,
    pub sent_at: Option<String>,
    pub attempts: i64,
    pub next_try_at: Option<String>,
    pub sending_at: Option<String>,
    /// `remote_machines.label` (NULL: no machine row, or no label).
    pub label: Option<String>,
}

pub(super) const OUTBOX_COLUMNS: &str = "o.request_id, o.machine_id, o.db_id, o.remote_id, o.qkey, o.action, \
     o.detail_json, o.created_at, o.expires_at, o.state, o.result, o.sent_at, o.attempts, o.next_try_at, o.sending_at";

impl OutboxRow {
    /// A row selected with [`OUTBOX_COLUMNS`] and then the label.
    pub(super) fn from_row(r: &Row<'_>) -> rusqlite::Result<OutboxRow> {
        Ok(OutboxRow {
            request_id: r.get(0)?,
            machine_id: r.get(1)?,
            db_id: r.get(2)?,
            remote_id: r.get(3)?,
            qkey: r.get(4)?,
            action: r.get(5)?,
            detail_json: r.get(6)?,
            created_at: r.get(7)?,
            expires_at: r.get(8)?,
            state: r.get(9)?,
            result: r.get(10)?,
            sent_at: r.get(11)?,
            attempts: r.get(12)?,
            next_try_at: r.get(13)?,
            sending_at: r.get(14)?,
            label: r.get(15)?,
        })
    }

    /// `<label>:Q<id>` (`A<id>` for an alert acknowledgement).
    pub fn reference(&self, label: &str) -> String {
        format!(
            "{label}:{}{}",
            if self.action == "ack_alert" { "A" } else { "Q" },
            self.remote_id
        )
    }

    /// Being sent right now (Python `_in_flight`).
    pub fn in_flight(&self, now: SystemTime) -> bool {
        self.sending_at
            .as_deref()
            .is_some_and(|s| !s.is_empty() && s > iso(now - SENDING_STALE).as_str())
    }
}

/// Why an action could not be queued (Python's `LookupError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gone(pub String);

/// The identity the operator saw: `data["id"]`, `data["qkey"]` (`akey` for
/// an alert) and `data["_db_id"]`.
fn shown<'a>(
    data: &'a Map<String, Value>,
    key: &str,
) -> (Option<i64>, Option<&'a str>, Option<&'a str>) {
    (
        data.get("id").and_then(py_int),
        data.get(key).and_then(Value::as_str),
        data.get("_db_id").and_then(Value::as_str),
    )
}

impl Store {
    /// Queues `action` on the mirrored question the operator saw (`data`, as
    /// shown, with `_db_id`): its request id. `Gone` when the mirror no longer
    /// holds that very question. `expiry`: `remote_answer_expiry_hours`.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &self,
        machine_id: &str,
        data: &Map<String, Value>,
        action: &str,
        detail: Option<&Value>,
        request_id: &str,
        now: SystemTime,
        expiry: Duration,
    ) -> rusqlite::Result<Result<String, Gone>> {
        let (id, qkey, db_id) = shown(data, "qkey");
        let row: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT db_id, qkey FROM remote_questions WHERE machine_id = ? AND remote_id = ?",
                params![machine_id, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((mirror_db, mirror_key)) = row else {
            return Ok(Err(Gone("that remote question is gone".into())));
        };
        if Some(mirror_key.as_str()) != qkey || Some(mirror_db.as_str()) != db_id {
            return Ok(Err(Gone("that question changed — reopen it".into())));
        }
        let detail_json = detail.filter(|d| py_truthy(d)).map(dumps);
        self.db().execute(
            "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, detail_json, created_at, \
             expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![request_id, machine_id, mirror_db, id, mirror_key, action, detail_json, iso(now), iso(now + expiry)],
        )?;
        Ok(Ok(request_id.to_string()))
    }

    /// Queues the operator's acknowledgement of the mirrored alert `data` (as
    /// shown); the mirror shows it acknowledged at once.
    pub fn enqueue_alert_ack(
        &self,
        machine_id: &str,
        data: &Map<String, Value>,
        request_id: &str,
        now: SystemTime,
        expiry: Duration,
    ) -> rusqlite::Result<Result<String, Gone>> {
        let (id, akey, db_id) = shown(data, "akey");
        let row: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT db_id, akey FROM remote_alerts WHERE machine_id = ? AND alert_id = ?",
                params![machine_id, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((mirror_db, mirror_key)) =
            row.filter(|(d, k)| Some(k.as_str()) == akey && Some(d.as_str()) == db_id)
        else {
            return Ok(Err(Gone(
                "that alert changed or is gone — reopen the inbox".into(),
            )));
        };
        self.db().execute(
            "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, detail_json, created_at, \
             expires_at) VALUES (?, ?, ?, ?, ?, 'ack_alert', NULL, ?, ?)",
            params![request_id, machine_id, mirror_db, id, mirror_key, iso(now), iso(now + expiry)],
        )?;
        self.db().execute(
            "UPDATE remote_alerts SET status = 'acked' WHERE machine_id = ? AND alert_id = ?",
            params![machine_id, id],
        )?;
        Ok(Ok(request_id.to_string()))
    }

    /// Moves a queued row to its final state, once (a compare-and-set on
    /// `queued`): the notice when it moved and `title` is given.
    pub(super) fn settle(
        &self,
        request_id: &str,
        state: &str,
        result: &str,
        notice: Option<(String, String)>,
        now: SystemTime,
    ) -> rusqlite::Result<(bool, Option<Notice>)> {
        let done = self.db().execute(
            "UPDATE remote_outbox SET state = ?, result = ?, sent_at = ? WHERE request_id = ? AND state = 'queued'",
            params![state, result, iso(now), request_id],
        )? == 1;
        Ok((
            done,
            notice
                .filter(|_| done)
                .map(|(title, body)| Notice { title, body }),
        ))
    }

    /// A queued action past its expiry: failed, with a notice that says
    /// whether it may have arrived (Python `_expire`).
    pub(super) fn expire(
        &self,
        row: &OutboxRow,
        label: &str,
        now: SystemTime,
    ) -> rusqlite::Result<Option<Notice>> {
        let body = if row.attempts != 0 {
            format!("it may have reached {label} before the connection dropped; check there")
        } else {
            format!("{label} was not reachable in time; answer again if still needed")
        };
        let title = format!(
            "{}: your queued {} expired",
            row.reference(label),
            row.action
        );
        Ok(self
            .settle(
                &row.request_id,
                "failed",
                "expired",
                Some((title, body)),
                now,
            )?
            .1)
    }

    /// Queued actions and ones that failed or were rejected, until seen (the popup).
    pub fn outbox_rows(&self) -> rusqlite::Result<Vec<OutboxRow>> {
        let mut stmt = self.db().prepare(&format!(
            "SELECT {OUTBOX_COLUMNS}, m.label FROM remote_outbox o LEFT JOIN remote_machines m USING (machine_id) \
             WHERE o.state IN ('queued', 'rejected', 'failed') ORDER BY o.created_at"
        ))?;
        stmt.query_map([], OutboxRow::from_row)?.collect()
    }

    /// The operator saw a failed or rejected action.
    pub fn mark_seen(&self, request_id: &str) -> rusqlite::Result<()> {
        self.db().execute(
            "UPDATE remote_outbox SET state = 'seen' WHERE request_id = ? AND state IN ('rejected', 'failed')",
            [request_id],
        )?;
        Ok(())
    }

    /// `d` in the popup: failed or rejected rows, and queued ones past expiry
    /// that are not being sent right now.
    pub fn clearable(&self, request_id: &str, now: SystemTime) -> rusqlite::Result<bool> {
        let row = self
            .db()
            .query_row(
                &format!(
                    "SELECT {OUTBOX_COLUMNS}, NULL FROM remote_outbox o WHERE o.request_id = ?"
                ),
                [request_id],
                OutboxRow::from_row,
            )
            .optional()?;
        Ok(match row {
            None => false,
            Some(r) if r.state == "rejected" || r.state == "failed" => true,
            Some(r) => r.state == "queued" && r.expires_at <= iso(now) && !r.in_flight(now),
        })
    }
}

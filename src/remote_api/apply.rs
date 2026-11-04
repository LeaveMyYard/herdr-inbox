//! `inbox apply` (Python `remote_api.apply`, `_ack_alert`, `_record`): one
//! operator action on one question or alert, idempotent by `request_id`.
//! **SAFETY**: the record of the request commits in the same transaction as
//! the action, so a retried request (its reply lost, or the process killed)
//! is answered from the record and never applied twice.

use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::detail::{Malformed, answer as parse_answer};
use super::keys::{akey, db_id, qkey};
use crate::clock::now;
use crate::pyjson::dumps;
use crate::remote::pyvalue::py_int;
use crate::store::Store;
use crate::store::questions::write_answer;

/// The actions a request may name (Python `ACTIONS`), besides `status`.
pub const ACTIONS: [&str; 5] = ["answer", "dismiss", "hide", "deliver", "ack_alert"];

/// What runs after the transaction commits (Python's `after`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum After {
    /// Refresh this pane's marks.
    Markers(Option<String>),
    /// Start a detached `inbox deliver <qid> [--force]`, then refresh the pane's marks
    /// (the answered question leaves the tab's count).
    Deliver {
        /// The question.
        qid: i64,
        /// Send now (the operator's "deliver"), not a fresh answer.
        force: bool,
        /// Its pane.
        pane: Option<String>,
    },
}

/// A reply, and what to do once it is committed.
pub type Reply = rusqlite::Result<(Value, Option<After>)>;

/// The reply to `request`, and what to do once it is committed.
pub fn apply(store: &Store, request: &Map<String, Value>) -> Reply {
    let request_id = request
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|r| (1..=64).contains(&r.chars().count()));
    let action = request.get("action").and_then(Value::as_str);
    if let (Some("status"), Some(rid)) = (action, request_id) {
        // asked after an expiry, instead of a late send: did this request apply? If not, it
        // never may: a tombstone, in the same locked transaction, so the original apply (still
        // running after the caller's ssh was killed) refuses if it commits later
        let reply = store.immediate(|db| {
            if let Some((outcome, reason)) = recorded(db, rid)? {
                return Ok(json!({"outcome": "recorded", "state": outcome, "reason": reason}));
            }
            record(db, rid, &Value::Null, "rejected", Some("expired"))?;
            Ok(json!({"outcome": "expired"}))
        })?;
        return Ok((reply, None));
    }
    let (Some(rid), Some(action)) = (request_id, action.filter(|a| ACTIONS.contains(a))) else {
        return Ok((
            json!({"outcome": "rejected", "reason": "bad request"}),
            None,
        ));
    };
    let qid = request.get("id").unwrap_or(&Value::Null);
    store.immediate(|db| {
        if let Some(done) = recorded(db, rid)? {
            return Ok((replayed(done), None));
        }
        let dbid = db_id(db)?;
        if request.get("db_id").and_then(Value::as_str) != Some(dbid.as_str()) {
            return Ok((record(db, rid, qid, "rejected", Some("reset"))?, None));
        }
        let key = request.get("qkey").and_then(Value::as_str);
        if action == "ack_alert" {
            return ack_alert(store, &dbid, rid, qid, key);
        }
        let q = match py_int(qid) {
            Some(id) => store.get(id)?,
            None => None,
        };
        let Some(q) = q.filter(|q| key == Some(qkey(&dbid, q).as_str())) else {
            return Ok((
                record(db, rid, qid, "rejected", Some("not that question"))?,
                None,
            ));
        };
        let reject =
            |reason: &str| -> Reply { Ok((record(db, rid, qid, "rejected", Some(reason))?, None)) };
        let after = match action {
            "answer" => {
                let empty = Map::new();
                let detail = request
                    .get("detail")
                    .and_then(Value::as_object)
                    .unwrap_or(&empty);
                let (text, detail) = match parse_answer(detail) {
                    Ok(Some(parsed)) => parsed,
                    Ok(None) => return reject("empty answer"),
                    Err(Malformed) => return reject("bad answer"), // Python: a traceback
                };
                if q.status == "answered" {
                    if q.answer.as_deref() == Some(text.as_str()) {
                        // ours, applied before a lost reply / a crash
                        record(db, rid, qid, "applied", None)?;
                        return Ok((json!({"outcome": "already_applied"}), None));
                    }
                    let was = q.answer.as_deref().unwrap_or("None");
                    return reject(&format!("answered with {was}"));
                }
                if q.status != "open" {
                    return reject(&q.status);
                }
                let wrote = write_answer(db, q.id, &text, Some(&dumps(&detail)))?;
                debug_assert!(wrote, "Q{} was read open under this same write lock", q.id);
                Some(After::Deliver {
                    qid: q.id,
                    force: false,
                    pane: q.pane_id.clone(),
                })
            }
            "dismiss" => {
                if q.status != "open" {
                    return reject(&q.status);
                }
                store.set_status(q.id, "dismissed", None)?;
                Some(After::Markers(q.pane_id.clone()))
            }
            "hide" => {
                store.hide_failed(q.id)?;
                None
            }
            _ => {
                // deliver: the operator's Send now of an answer not delivered yet
                if q.status != "answered"
                    || q.delivered_at.as_deref().is_some_and(|d| !d.is_empty())
                {
                    return reject("nothing to deliver");
                }
                Some(After::Deliver {
                    qid: q.id,
                    force: true,
                    pane: q.pane_id.clone(),
                })
            }
        };
        Ok((record(db, rid, qid, "applied", None)?, after))
    })
}

/// The operator acknowledged a remote alert (Python `_ack_alert`), inside
/// the request's transaction.
fn ack_alert(store: &Store, dbid: &str, rid: &str, aid: &Value, key: Option<&str>) -> Reply {
    let db = store.db();
    let alert = match py_int(aid) {
        Some(id) => store.alert(id)?,
        None => None,
    };
    let Some(a) = alert.filter(|a| key == Some(akey(dbid, a).as_str())) else {
        return Ok((
            record(db, rid, aid, "rejected", Some("not that alert"))?,
            None,
        ));
    };
    if a.status != "open" {
        return Ok((record(db, rid, aid, "rejected", Some(&a.status))?, None));
    }
    db.execute(
        "UPDATE alerts SET status = 'acked', acked_at = ? WHERE id = ?",
        params![now(), a.id],
    )?;
    Ok((
        record(db, rid, aid, "applied", None)?,
        Some(After::Markers(a.pane_id)),
    ))
}

/// The recorded outcome and reason of `rid`, if it was recorded.
fn recorded(db: &Connection, rid: &str) -> rusqlite::Result<Option<(String, Option<String>)>> {
    db.query_row(
        "SELECT outcome, reason FROM applied_requests WHERE request_id = ?",
        [rid],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// The reply to a request already recorded.
fn replayed((outcome, reason): (String, Option<String>)) -> Value {
    if outcome == "applied" {
        json!({"outcome": "already_applied"})
    } else {
        json!({"outcome": outcome, "reason": reason})
    }
}

/// Records `rid`'s outcome (Python `_record`) and returns the reply.
fn record(
    db: &Connection,
    rid: &str,
    qid: &Value,
    outcome: &str,
    reason: Option<&str>,
) -> rusqlite::Result<Value> {
    db.execute(
        "INSERT INTO applied_requests (request_id, qid, outcome, reason, at) VALUES (?, ?, ?, ?, ?)",
        params![rid, bound(qid), outcome, reason, now()],
    )?;
    // Python `if reason`: an empty reason is left out of the reply (it is still recorded)
    Ok(match reason.filter(|r| !r.is_empty()) {
        Some(reason) => json!({"outcome": outcome, "reason": reason}),
        None => json!({"outcome": outcome}),
    })
}

/// The request's `id` as Python's sqlite3 binds it (`qid INTEGER` then
/// converts a numeric text or a whole real, as SQLite does for both). A
/// list or object, which Python fails to bind (a traceback, nothing
/// recorded), is recorded as NULL; so is an integer beyond 64 bits (Python:
/// `OverflowError`).
fn bound(qid: &Value) -> Sql {
    match qid {
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => match (n.as_i64(), n.is_f64()) {
            (Some(i), _) => Sql::Integer(i),
            (None, true) => n.as_f64().map_or(Sql::Null, Sql::Real),
            (None, false) => Sql::Null,
        },
        Value::String(s) => Sql::Text(s.clone()),
        _ => Sql::Null,
    }
}

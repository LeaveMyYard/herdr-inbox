//! `inbox export` (Python `remote_api.export`, `_row`, `_alerts`): the
//! questions changed since the puller's cursor, or a full snapshot, and the
//! alerts. No data writes, except the database's id the first time.

use std::time::{Duration, SystemTime};

use rusqlite::OptionalExtension;
use serde_json::{Map, Value, json};

use super::keys::{akey, db_id, qkey};
use crate::clock::iso_seconds;
use crate::remote::pyvalue::{py_int, py_truthy};
use crate::store::question::{COLUMNS, QOption, VIEW};
use crate::store::{Question, Store};

/// The export format this side writes (Python `EXPORT_VERSION`).
pub const EXPORT_VERSION: i64 = 1;
/// How long an answered question the agent has not seen stays in a full
/// snapshot (Python `RECENT`).
pub const RECENT: Duration = Duration::from_secs(7 * 86_400);
/// How long a closed alert is still sent, so the mirror sees it close.
pub const ALERTS_CLOSED_FOR: Duration = Duration::from_secs(3600);
/// The most alerts one export sends.
pub const MAX_ALERTS: i64 = 200;

/// The export for `request` (`{"cursor": {"db_id", "seq"}, "full"}`), with
/// the windows measured from `now`. Python's types decide: a cursor that is
/// not an object is none, a `seq` is an int when Python's `isinstance(seq,
/// int)` says so (a bool is one), `full` is its truth value.
pub fn export(
    store: &Store,
    request: &Map<String, Value>,
    now: SystemTime,
) -> rusqlite::Result<Value> {
    let db = store.db();
    let dbid = db_id(db)?;
    let seq: i64 = db.query_row("SELECT COALESCE(MAX(seq), 0) FROM changes", [], |r| {
        r.get(0)
    })?;
    let oldest: i64 = db.query_row("SELECT COALESCE(MIN(seq), 0) FROM changes", [], |r| {
        r.get(0)
    })?;
    let cursor = request.get("cursor").and_then(Value::as_object);
    // the cursor's seq when it is usable: this database's, and not pruned past
    let since = cursor
        .filter(|c| c.get("db_id").and_then(Value::as_str) == Some(dbid.as_str()))
        .and_then(|c| c.get("seq").and_then(py_int))
        .filter(|&s| s >= oldest - 1);
    let full = request.get("full").is_some_and(py_truthy) || since.is_none();
    let (qs, gone) = match since.filter(|_| !full) {
        None => (full_snapshot(store, now)?, None),
        Some(since) => {
            let (qs, gone) = changed_since(store, since)?;
            (qs, Some(gone))
        }
    };
    let questions = qs
        .iter()
        .map(|q| row(store, q, &dbid))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Map::new();
    out.insert("version".into(), env!("CARGO_PKG_VERSION").into());
    out.insert("format".into(), EXPORT_VERSION.into());
    out.insert("db_id".into(), dbid.clone().into());
    out.insert("seq".into(), seq.into());
    out.insert("full".into(), full.into());
    out.insert("questions".into(), Value::Array(questions));
    out.insert("alerts".into(), Value::Array(alerts(store, &dbid, now)?));
    match gone {
        None => out.insert(
            "ids".into(),
            json!(qs.iter().map(|q| q.id).collect::<Vec<_>>()),
        ),
        Some(gone) => out.insert("gone".into(), json!(gone)), // changed and no longer there (deleted)
    };
    Ok(Value::Object(out))
}

/// Every open question, and every answer of the last [`RECENT`] the agent has not seen.
fn full_snapshot(store: &Store, now: SystemTime) -> rusqlite::Result<Vec<Question>> {
    let mut stmt = store.db().prepare(&format!(
        "SELECT {COLUMNS} FROM {VIEW} WHERE status = 'open' \
         OR (status = 'answered' AND acknowledged_at IS NULL AND answered_at >= ?) ORDER BY id"
    ))?;
    let rows = stmt.query_map([iso_seconds(now - RECENT)], Question::from_row)?;
    rows.collect()
}

/// The questions changed after `since`, and the ids of those changed and gone.
fn changed_since(store: &Store, since: i64) -> rusqlite::Result<(Vec<Question>, Vec<i64>)> {
    let mut stmt = store
        .db()
        .prepare("SELECT DISTINCT qid FROM changes WHERE seq > ? ORDER BY qid")?;
    let ids = stmt
        .query_map([since], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (mut qs, mut gone) = (Vec::new(), Vec::new());
    for id in ids {
        match store.get(id)? {
            Some(q) => qs.push(q),
            None => gone.push(id),
        }
    }
    Ok((qs, gone))
}

/// One question as the puller reads it (Python `_row`, key for key).
fn row(store: &Store, q: &Question, dbid: &str) -> rusqlite::Result<Value> {
    let ext: Option<(bool, bool)> = store
        .db()
        .query_row(
            "SELECT hidden_at, agent_gone_at FROM question_ext WHERE id = ?",
            [q.id],
            |r| Ok((truthy_text(r.get(0)?), truthy_text(r.get(1)?))),
        )
        .optional()?;
    let (hidden, agent_gone) = ext.unwrap_or_default();
    let options: Vec<Value> = q
        .options
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(option)
        .collect();
    Ok(json!({
        "id": q.id, "qkey": qkey(dbid, q), "group": q.group(), "status": q.status,
        "question": q.question, "header": q.header, "options": options,
        "multi_select": q.multi_select, "tab_label": q.tab_label, "pane_id": q.pane_id,
        "asked_by": q.asked_by, "title": q.group_title, "created_at": q.created_at,
        "answered_at": q.answered_at, "answer": q.answer, "delivery_error": q.delivery_error,
        "delivered_at": q.delivered_at, "verified_at": q.verified_at,
        "acknowledged_at": q.acknowledged_at, "delivery_attempts": q.delivery_attempts,
        "hidden": hidden, "agent_gone": agent_gone,
    }))
}

/// Python's `bool(row[i])` of a column the inbox fills with a time or NULL.
fn truthy_text(v: rusqlite::types::Value) -> bool {
    use rusqlite::types::Value as V;
    match v {
        V::Null => false,
        V::Integer(n) => n != 0,
        V::Real(f) => f != 0.0,
        V::Text(s) => !s.is_empty(),
        V::Blob(b) => !b.is_empty(),
    }
}

/// An option as Python's `normalize_options` leaves it.
fn option(o: &QOption) -> Value {
    json!({"label": o.label, "description": o.description, "preview": o.preview})
}

/// Open alerts, and those closed in the last hour (Python `_alerts`): small,
/// sent whole on every pull.
fn alerts(store: &Store, dbid: &str, now: SystemTime) -> rusqlite::Result<Vec<Value>> {
    let since = iso_seconds(now - ALERTS_CLOSED_FOR);
    Ok(store
        .exported_alerts(&since, MAX_ALERTS)?
        .iter()
        .map(|a| {
            json!({
                "id": a.id, "akey": akey(dbid, a), "kind": a.kind, "severity": a.severity,
                "title": a.title, "message": a.message, "tab_label": a.tab_label,
                "pane_id": a.pane_id, "asked_by": a.asked_by, "status": a.status,
                "created_at": a.created_at,
            })
        })
        .collect())
}

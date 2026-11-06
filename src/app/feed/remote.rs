//! The remote machines' rows, from the local mirror. They are read-only in
//! v1: no token, no action.

use std::time::SystemTime;

use serde_json::{Map, Value, json};

use super::group::option;
use super::text::{LONG, Text};
use crate::remote::pyvalue::py_truthy;
use crate::remote::validate::py_int;
use crate::store::Store;
use crate::store::mirror::{Machine, iso};
use crate::store::question::normalize_options;

/// The remote rows in their kinds, each kind in the feed's order.
#[derive(Debug, Default)]
pub struct Rows {
    /// `remote_alert` rows.
    pub alerts: Vec<Value>,
    /// `remote_group` rows.
    pub groups: Vec<Value>,
    /// `remote_outbox` rows.
    pub outbox: Vec<Value>,
    /// `remote_problem` rows.
    pub problems: Vec<Value>,
    /// Mirrored rows with no id, which nothing can name.
    pub unreadable: usize,
}

/// Reads the mirror at wall-clock time `wall`.
pub fn read(store: &Store, wall: SystemTime) -> rusqlite::Result<Rows> {
    let machines = store.machines_by_label()?;
    let mut rows = Rows::default();
    for (m, a) in store.mirror_open_alerts()? {
        match alert(&m, &a, wall) {
            Some(row) => rows.alerts.push(row),
            None => rows.unreadable += 1,
        }
    }
    for (key, m, qs) in store.mirror_open_groups()? {
        let questions: Vec<Value> = qs.iter().filter_map(question).collect();
        rows.unreadable += qs.len() - questions.len();
        if questions.is_empty() {
            continue;
        }
        let first = &qs[0];
        rows.groups.push(sealed(json!({
            "key": format!("remote:{key}"),
            "kind": "remote_group",
            "machine": machine(&m, wall),
            "title": text(first, "title"),
            "created_at": text(first, "created_at").filter(|t| !t.is_empty()),
            "tab_label": text(first, "tab_label"),
            "questions": questions,
        })));
    }
    for o in store.outbox_rows()? {
        let queued = o.state == "queued";
        let sending = queued && o.sending_at.as_deref().is_some_and(|s| !s.is_empty());
        rows.outbox.push(sealed(json!({
            "key": format!("routbox:{}", o.request_id),
            "kind": "remote_outbox",
            "machine": known(&machines, &o.machine_id, wall),
            "action": o.action,
            "remote_id": o.remote_id,
            "state": o.state,
            "result": o.result,
            "sending": sending,
            "expired": queued && o.expires_at <= iso(wall),
            "expires_at": o.expires_at,
        })));
    }
    for p in store.problem_rows()? {
        let mut cut = false;
        let field = |key: &str| p.data.get(key).and_then(Value::as_str).map(str::to_string);
        let question = Text::Cut.apply(&field("question").unwrap_or_default(), LONG, &mut cut);
        let answer = Text::Cut.apply_opt(field("answer").as_deref(), LONG, &mut cut);
        rows.problems.push(sealed(json!({
            "key": format!("rproblem:{}|{}", p.machine_id, p.remote_id),
            "kind": "remote_problem",
            "machine": known(&machines, &p.machine_id, wall),
            "remote_id": p.remote_id,
            "why": p.why,
            "question": question,
            "answer": answer,
            "cut": cut,
        })));
    }
    Ok(rows)
}

/// A remote row offers nothing.
fn sealed(mut row: Value) -> Value {
    row["actions"] = json!([]);
    row["token"] = Value::Null;
    row
}

fn text(data: &Map<String, Value>, key: &str) -> Option<String> {
    data.get(key).and_then(Value::as_str).map(str::to_string)
}

fn machine(m: &Machine, wall: SystemTime) -> Value {
    json!({"id": m.machine_id, "label": m.label, "offline": m.offline_note(wall)})
}

/// Machine `id`. A row can outlive its machine's mirror row: it still names it.
fn known(machines: &[Machine], id: &str, wall: SystemTime) -> Value {
    match machines.iter().find(|m| m.machine_id == id) {
        Some(m) => machine(m, wall),
        None => json!({"id": id, "label": null, "offline": null}),
    }
}

fn alert(m: &Machine, data: &Map<String, Value>, wall: SystemTime) -> Option<Value> {
    let id = data.get("id").and_then(py_int)?;
    let or = |key, default: &str| {
        text(data, key)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default.to_string())
    };
    Some(sealed(json!({
        "key": format!("ralert:{}|{id}", m.machine_id),
        "kind": "remote_alert",
        "machine": machine(m, wall),
        "id": id,
        "alert_kind": or("kind", "agent"),
        "severity": or("severity", "warning"),
        "title": or("title", ""),
        "message": text(data, "message"),
        "tab_label": text(data, "tab_label"),
        "asked_by": text(data, "asked_by"),
        "created_at": text(data, "created_at").filter(|t| !t.is_empty()),
    })))
}

/// A `Question` of a remote row. Its `draft` is always null.
fn question(data: &Map<String, Value>) -> Option<Value> {
    let id = data.get("id").and_then(py_int)?;
    let mut cut = false;
    let options = data
        .get("options")
        .filter(|o| py_truthy(o))
        .and_then(|o| normalize_options(o).ok().flatten())
        .unwrap_or_default();
    let options: Vec<Value> = options
        .iter()
        .map(|o| option(o, Text::Cut, &mut cut))
        .collect();
    let body = Text::Cut.apply(&text(data, "question").unwrap_or_default(), LONG, &mut cut);
    let context = Text::Cut.apply_opt(text(data, "context").as_deref(), LONG, &mut cut);
    Some(json!({
        "id": id,
        "header": text(data, "header"),
        "question": body,
        "options": options,
        "multi_select": data.get("multi_select").is_some_and(py_truthy),
        "context": context,
        "asked_by": text(data, "asked_by"),
        "draft": null,
        "cut": cut,
    }))
}

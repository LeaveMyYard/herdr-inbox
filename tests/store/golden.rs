//! What the store golden replays share (tests/fixtures/store-*.json, from
//! tools/capture_goldens.py --only store): Python's values as the capture
//! wrote them, times masked the same way, and the raw-SQL setup step.

use herdr_inbox::store::Store;
use herdr_inbox::store::question::{QOption, Question};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};

/// A time a step sets (capture_goldens `STORE_FIXED`): never masked.
pub const FIXED: &str = "2001-01-01T00:00:00+00:00";

/// Exactly `YYYY-MM-DDTHH:MM:SS+00:00` (Python's capture masks only that: the format is pinned too).
fn is_time(s: &str) -> bool {
    let shape = "dddd-dd-ddTdd:dd:dd+00:00";
    s.len() == shape.len()
        && s.bytes().zip(shape.bytes()).all(|(c, want)| {
            if want == b'd' {
                c.is_ascii_digit()
            } else {
                c == want
            }
        })
}

/// A string as Python's capture wrote it: a time (other than [`FIXED`]) as
/// `<time>`, any claim `sending:<time>+00:00:<8 lowercase hex>` as
/// `sending:<time>:<token>`.
pub fn mask(s: &str) -> Value {
    if is_time(s) && s != FIXED {
        return Value::String("<time>".into());
    }
    if let Some((at, token)) = s
        .strip_prefix("sending:")
        .and_then(|rest| rest.rsplit_once(':'))
        && is_time(at)
        && token.len() == 8
        && token
            .bytes()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Value::String("sending:<time>:<token>".into());
    }
    Value::String(s.into())
}

pub fn opt(v: &Option<String>) -> Value {
    v.as_deref().map_or(Value::Null, mask)
}

pub fn options(o: &Option<Vec<QOption>>) -> Value {
    o.as_ref().map_or(Value::Null, |list| {
        Value::Array(
            list.iter()
                .map(|q| json!({"label": q.label, "description": q.description, "preview": q.preview}))
                .collect(),
        )
    })
}

/// `dataclasses.asdict(question)` as the capture wrote it: every string masked
/// (times and claims), however deep.
pub fn question(q: &Question) -> Value {
    deep_mask(json!({
        "id": q.id, "created_at": mask(&q.created_at), "pane_id": q.pane_id, "tab_id": q.tab_id,
        "tab_label": q.tab_label, "session_id": q.session_id, "cwd": q.cwd, "asked_by": q.asked_by,
        "question": q.question, "options": options(&q.options), "context": q.context, "norm": q.norm,
        "status": q.status, "answer": q.answer, "answered_at": opt(&q.answered_at),
        "delivered_at": opt(&q.delivered_at), "delivery_error": q.delivery_error,
        "acknowledged_at": opt(&q.acknowledged_at), "repeat_count": q.repeat_count,
        "delivery_attempts": q.delivery_attempts, "header": q.header, "multi_select": q.multi_select,
        "answer_json": q.answer_json, "group_id": q.group_id, "draft_json": q.draft_json,
        "drafted_at": opt(&q.drafted_at), "group_title": q.group_title, "verified_at": opt(&q.verified_at),
    }))
}

/// Python's `_store_value` on a JSON value: [`mask`] on every string.
pub fn deep_mask(v: Value) -> Value {
    match v {
        Value::String(s) => mask(&s),
        Value::Array(items) => Value::Array(items.into_iter().map(deep_mask).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, deep_mask(v))).collect()),
        other => other,
    }
}

pub fn tables(store: &Store, names: &[&str]) -> Value {
    let mut out = Map::new();
    for table in names {
        let mut stmt = store
            .db()
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Value> = stmt
            .query_map([], |r| {
                let mut row = Map::new();
                for (i, name) in names.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => json!(n),
                        ValueRef::Real(f) => json!(f),
                        ValueRef::Text(t) => mask(std::str::from_utf8(t).unwrap()),
                        ValueRef::Blob(_) => panic!("blob"),
                    };
                    row.insert(name.clone(), v);
                }
                Ok(Value::Object(row))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        out.insert((*table).into(), Value::Array(rows));
    }
    Value::Object(out)
}

pub fn s<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str)
}

pub fn i(args: &Value, k: &str) -> i64 {
    args[k].as_i64().unwrap()
}

pub fn list(qs: Vec<Question>) -> Value {
    Value::Array(qs.iter().map(question).collect())
}

/// A `sql` step: raw SQL that sets a row up (fixed times, states no call makes).
pub fn sql(store: &Store, a: &Value) {
    let params: Vec<rusqlite::types::Value> = a["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| match v {
            Value::String(s) => rusqlite::types::Value::Text(s.clone()),
            Value::Number(n) => rusqlite::types::Value::Integer(n.as_i64().unwrap()),
            Value::Null => rusqlite::types::Value::Null,
            other => panic!("param {other}"),
        })
        .collect();
    store
        .db()
        .execute(s(a, "sql").unwrap(), rusqlite::params_from_iter(params))
        .unwrap();
}

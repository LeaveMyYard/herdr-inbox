//! The remote mirror replays tests/fixtures/store-mirror.json: Python's
//! `remote.py` over `STORE_MIRROR_OPS` (pulls with the ssh stubbed: failures
//! and backoff, full and incremental snapshots, a reset, alerts being
//! acknowledged; the popup's reads; stuck answers), with the four mirror
//! tables after each step, the export call's cursor and every notification;
//! and `validate_export` over `STORE_EXPORT_VECTORS`. Time is injected, so
//! nothing is masked.

use std::cell::RefCell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_inbox::pyjson;
use herdr_inbox::remote::validate::{Export, ExportError, py_int, validate_export};
use herdr_inbox::store::Store;
use herdr_inbox::store::mirror::{CallResult, Machine, Notice};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};

use crate::common::migrated;
use crate::golden::{i, s, sql};

const GOLDEN: &str = include_str!("../fixtures/store-mirror.json");

fn at(a: &Value) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(a["now"].as_u64().unwrap())
}

fn machine(m: &Machine, with_id: bool) -> Value {
    let mut v = json!({
        "machine_id": m.machine_id, "label": m.label, "target": m.target, "db_id": m.db_id,
        "cursor_seq": m.cursor_seq, "version": m.version, "uv": m.uv, "herdr": m.herdr,
        "plugin_root": m.plugin_root, "pulls": m.pulls, "failures": m.failures, "last_ok_at": m.last_ok_at,
        "last_error": m.last_error, "last_try_at": m.last_try_at, "backoff_until": m.backoff_until,
    });
    if with_id {
        v["id"] = json!(m.machine_id);
    }
    v
}

fn tables(store: &Store) -> Value {
    let mut out = Map::new();
    for table in [
        "remote_machines",
        "remote_questions",
        "remote_alerts",
        "remote_outbox",
    ] {
        let mut stmt = store
            .db()
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Value> = stmt
            .query_map([], |r| {
                let mut row = Map::new();
                for (i, name) in names.iter().enumerate() {
                    row.insert(
                        name.clone(),
                        match r.get_ref(i)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => json!(n),
                            ValueRef::Real(f) => json!(f),
                            ValueRef::Text(t) => json!(std::str::from_utf8(t).unwrap()),
                            ValueRef::Blob(_) => panic!("blob"),
                        },
                    );
                }
                Ok(Value::Object(row))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        out.insert(table.into(), Value::Array(rows));
    }
    Value::Object(out)
}

fn notices(effects: &mut Vec<Value>, list: Vec<Notice>) {
    effects.extend(list.into_iter().map(|n| json!(["notify", n.title, n.body])));
}

/// One step: its result and effects.
fn run(store: &Store, op: &str, a: &Value) -> (Value, Vec<Value>) {
    let mut effects = Vec::new();
    let result = match op {
        "sql" => {
            sql(store, a);
            Value::Null
        }
        "pull" => {
            let m = &a["machine"];
            let calls = RefCell::new(Vec::new());
            let (ok, got) = store
                .pull(
                    m["id"].as_str().unwrap(),
                    m["label"].as_str(),
                    m["target"].as_str(),
                    at(a),
                    |rec| match a.get("paths_error").and_then(Value::as_str) {
                        Some(e) => Err(e.to_string()),
                        None => Ok(rec.clone()),
                    },
                    |_, cursor, full| {
                        let cursor = cursor.map_or(Value::Null, |(db, seq)| json!({"db_id": db, "seq": seq}));
                        calls.borrow_mut().push(json!(["call", cursor, full]));
                        CallResult {
                            code: a.get("code").and_then(Value::as_i64).unwrap_or(0) as i32,
                            stdout: a.get("export").map_or(String::new(), pyjson::dumps),
                            stderr: s(a, "stderr").unwrap_or("").into(),
                        }
                    },
                )
                .unwrap();
            effects.extend(calls.into_inner());
            notices(&mut effects, got);
            json!(ok)
        }
        "open_alerts" => Value::Array(
            store.mirror_open_alerts().unwrap().iter().map(|(m, a)| json!([machine(m, true), a])).collect(),
        ),
        "alert_by_key" => store
            .mirror_alert_by_key(s(a, "key").unwrap())
            .unwrap()
            .map_or(Value::Null, |(m, a)| json!([machine(&m, true), a])),
        "open_groups" => Value::Array(
            store.mirror_open_groups().unwrap().iter().map(|(k, m, qs)| json!([k, machine(m, false), qs])).collect(),
        ),
        "group_by_key" => store
            .mirror_group_by_key(s(a, "key").unwrap())
            .unwrap()
            .map_or(Value::Null, |(m, qs)| json!([machine(&m, false), qs])),
        "problem_rows" => Value::Array(
            store
                .problem_rows()
                .unwrap()
                .into_iter()
                .map(|p| json!({"machine_id": p.machine_id, "remote_id": p.remote_id, "label": p.label, "why": p.why, "data": p.data}))
                .collect(),
        ),
        "clear_problem" => {
            store.clear_problem(s(a, "machine_id").unwrap(), i(a, "remote_id"), at(a)).unwrap();
            Value::Null
        }
        "notify_problems" => {
            notices(&mut effects, store.notify_problems(s(a, "machine_id").unwrap(), s(a, "label").unwrap(), at(a)).unwrap());
            Value::Null
        }
        other => panic!("no Rust call for {other}"),
    };
    (result, effects)
}

#[test]
fn every_mirror_step_leaves_pythons_rows_and_sends_pythons_notices() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    let steps = golden["steps"].as_array().unwrap();
    assert!(steps.len() >= 30);
    for (n, step) in steps.iter().enumerate() {
        let op = step["op"].as_str().unwrap();
        let (result, effects) = run(&store, op, &step["args"]);
        assert_eq!(result, step["result"], "step {n} {op}: result");
        assert_eq!(
            Value::Array(effects),
            step["effects"],
            "step {n} {op}: effects"
        );
        assert_eq!(tables(&store), step["tables"], "step {n} {op}: tables");
    }
}

/// Python's returned dict, with its ints as Python sees them (`True` is 1).
fn export_json(e: &Export) -> Value {
    json!({"db_id": e.db_id, "seq": e.seq, "full": e.full, "version": e.version, "ids": e.ids, "gone": e.gone,
           "questions": e.questions, "alerts": e.alerts})
}

fn ints_of(v: &Value) -> Value {
    match v {
        Value::Array(list) => Value::Array(
            list.iter()
                .map(|x| py_int(x).map_or(x.clone(), Value::from))
                .collect(),
        ),
        other => py_int(other).map_or(other.clone(), Value::from),
    }
}

#[test]
fn exports_validate_as_pythons() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let vectors = golden["vectors"].as_array().unwrap();
    assert!(vectors.len() >= 20);
    for v in vectors {
        let raw = v["raw"].as_str().unwrap();
        let got = validate_export(raw);
        match v.get("error").and_then(Value::as_str) {
            Some(kind) => {
                let (got_kind, msg) = match got {
                    Err(ExportError::Format(m)) => ("PermissionError", m),
                    Err(ExportError::Bad(m)) => ("ValueError", m),
                    Ok(e) => panic!("{raw:.120}: accepted {e:?}"),
                };
                assert_eq!(
                    (got_kind, msg.as_str()),
                    (kind, v["message"].as_str().unwrap()),
                    "{raw:.200}"
                );
            }
            None => {
                let mut want = v["result"].clone();
                for k in ["seq", "ids", "gone"] {
                    want[k] = ints_of(&want[k]);
                }
                let mut got = export_json(&got.unwrap_or_else(|e| panic!("{raw:.120}: {e:?}")));
                if want["full"] == json!(false) {
                    // Python passes a non-full export's `ids` through unchecked; only a full one reads it
                    want["ids"] = Value::Null;
                    got["ids"] = Value::Null;
                }
                assert_eq!(got, want, "{raw:.200}");
            }
        }
    }
}

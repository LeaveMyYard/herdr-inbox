//! The question reads and writes replay tests/fixtures/store-questions.json:
//! Python's `store.Store` over a script of calls, with every touched table
//! dumped after each step. The Rust store runs the same script on a fresh
//! database built from the captured schema and must leave the same rows.

use herdr_inbox::store::Store;
use herdr_inbox::store::question::{OptionsError, QOption, Question, normalize_options};
use herdr_inbox::store::questions::{NewQuestion, Owner};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};

use crate::common::migrated;
use crate::family::Fake;

const GOLDEN: &str = include_str!("../fixtures/store-questions.json");
/// A time a step sets (capture_goldens `STORE_FIXED`): never masked.
const FIXED: &str = "2001-01-01T00:00:00+00:00";

/// A time as Python's capture masked it.
fn mask(s: &str) -> Value {
    // exactly YYYY-MM-DDTHH:MM:SS+00:00, as Python's capture masks it: the format is pinned too
    let shape = "dddd-dd-ddTdd:dd:dd+00:00";
    let iso = s.len() == shape.len()
        && s.bytes().zip(shape.bytes()).all(|(c, want)| {
            if want == b'd' {
                c.is_ascii_digit()
            } else {
                c == want
            }
        });
    if iso && s != FIXED {
        return Value::String("<time>".into());
    }
    if let Some(rest) = s.strip_prefix("sending:") {
        // sending:<iso>+00:00:<8 hex>
        if let Some((at, token)) = rest.rsplit_once(':')
            && token.len() == 8
            && token.bytes().all(|c| c.is_ascii_hexdigit())
            && mask(at) == Value::String("<time>".into())
        {
            return Value::String("sending:<time>:<token>".into());
        }
    }
    Value::String(s.into())
}

fn opt(v: &Option<String>) -> Value {
    v.as_deref().map_or(Value::Null, mask)
}

fn options(o: &Option<Vec<QOption>>) -> Value {
    o.as_ref().map_or(Value::Null, |list| {
        Value::Array(
            list.iter()
                .map(|q| json!({"label": q.label, "description": q.description, "preview": q.preview}))
                .collect(),
        )
    })
}

/// `dataclasses.asdict(question)` with times masked.
fn question(q: &Question) -> Value {
    json!({
        "id": q.id, "created_at": mask(&q.created_at), "pane_id": q.pane_id, "tab_id": q.tab_id,
        "tab_label": q.tab_label, "session_id": q.session_id, "cwd": q.cwd, "asked_by": q.asked_by,
        "question": q.question, "options": options(&q.options), "context": q.context, "norm": q.norm,
        "status": q.status, "answer": q.answer, "answered_at": opt(&q.answered_at),
        "delivered_at": opt(&q.delivered_at), "delivery_error": q.delivery_error,
        "acknowledged_at": opt(&q.acknowledged_at), "repeat_count": q.repeat_count,
        "delivery_attempts": q.delivery_attempts, "header": q.header, "multi_select": q.multi_select,
        "answer_json": q.answer_json, "group_id": q.group_id, "draft_json": q.draft_json,
        "drafted_at": opt(&q.drafted_at), "group_title": q.group_title, "verified_at": opt(&q.verified_at),
    })
}

fn tables(store: &Store) -> Value {
    let mut out = Map::new();
    for table in ["questions", "question_ext", "question_groups"] {
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
        out.insert(table.into(), Value::Array(rows));
    }
    Value::Object(out)
}

fn s<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str)
}

fn i(args: &Value, k: &str) -> i64 {
    args[k].as_i64().unwrap()
}

fn list(qs: Vec<Question>) -> Value {
    Value::Array(qs.iter().map(question).collect())
}

/// One step of the script, as the Rust store runs it; its result as Python returned it.
fn run(store: &Store, op: &str, a: &Value) -> Value {
    let files = Fake::default();
    let owner = Owner {
        session_id: s(a, "session_id"),
        pane_id: s(a, "pane_id"),
    };
    let closed = a
        .get("include_closed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let detail = a.get("detail").filter(|d| !d.is_null());
    match op {
        "insert" => {
            let new = NewQuestion {
                question: s(a, "question").unwrap(),
                options: a.get("options").and_then(|o| normalize_options(o).unwrap()),
                context: s(a, "context"),
                asked_by: s(a, "asked_by"),
                pane_id: s(a, "pane_id"),
                tab_id: s(a, "tab_id"),
                tab_label: s(a, "tab_label"),
                session_id: s(a, "session_id"),
                cwd: s(a, "cwd"),
                header: s(a, "header"),
                multi_select: a
                    .get("multi_select")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                group_id: a.get("group_id").and_then(Value::as_i64),
            };
            question(&store.insert(&new).unwrap())
        }
        "set_group_title" => {
            store
                .set_group_title(i(a, "group_id"), s(a, "title"))
                .unwrap();
            Value::Null
        }
        "find_duplicate" => store
            .find_duplicate(s(a, "norm").unwrap(), owner, &files)
            .unwrap()
            .map_or(Value::Null, |q| question(&q)),
        "set_draft" => json!(store.set_draft(i(a, "qid"), detail).unwrap()),
        "drafted" => list(store.drafted().unwrap()),
        "set_answer" => json!(
            store
                .set_answer(i(a, "qid"), s(a, "answer").unwrap(), detail)
                .unwrap()
        ),
        "set_status" => json!(
            store
                .set_status(i(a, "qid"), s(a, "status").unwrap(), s(a, "note"))
                .unwrap()
        ),
        "bump_repeat" => {
            store.bump_repeat(i(a, "qid")).unwrap();
            Value::Null
        }
        "mine" => list(store.mine(owner, closed, &files).unwrap()),
        "list" => list(store.list(closed).unwrap()),
        "group" => list(store.group(i(a, "group_id"), closed).unwrap()),
        "open_counts" => json!(store.open_counts().unwrap()),
        "known_panes" => json!(store.known_panes().unwrap()),
        "unacknowledged_answers" => list(store.unacknowledged_answers(owner, &files).unwrap()),
        "acknowledge" => {
            let ids: Vec<i64> = serde_json::from_value(a["qids"].clone()).unwrap();
            store.acknowledge(&ids).unwrap();
            Value::Null
        }
        "redeliver" => {
            let ids: Vec<i64> = serde_json::from_value(a["qids"].clone()).unwrap();
            json!(store.redeliver(&ids).unwrap())
        }
        "hide_failed" => {
            store.hide_failed(i(a, "qid")).unwrap();
            Value::Null
        }
        "sql" => {
            let params: Vec<rusqlite::types::Value> = a["params"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| match v {
                    Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                    Value::Number(n) => rusqlite::types::Value::Integer(n.as_i64().unwrap()),
                    other => panic!("param {other}"),
                })
                .collect();
            store
                .db()
                .execute(s(a, "sql").unwrap(), rusqlite::params_from_iter(params))
                .unwrap();
            Value::Null
        }
        "claim_delivery" => store
            .claim_delivery(i(a, "qid"))
            .unwrap()
            .map_or(Value::Null, |c| mask(&c.0)),
        "get" => store
            .get(i(a, "qid"))
            .unwrap()
            .map_or(Value::Null, |q| question(&q)),
        other => panic!("no Rust call for {other}"),
    }
}

#[test]
fn every_step_leaves_pythons_rows_and_returns_pythons_result() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), std::time::Duration::from_secs(5)).unwrap();
    let steps = golden["steps"].as_array().unwrap();
    assert!(steps.len() >= 25);
    for (n, step) in steps.iter().enumerate() {
        let op = step["op"].as_str().unwrap();
        let got = run(&store, op, &step["args"]);
        assert_eq!(got, step["result"], "step {n} {op}: result");
        assert_eq!(tables(&store), step["tables"], "step {n} {op}: tables");
    }
}

/// What Python's `normalize_options` makes of each input, or which exception
/// it raises. A truthy non-string description or preview is kept as a value by
/// Python and as its `str()` spelling here (documented in `normalize_options`).
#[test]
fn options_normalize_as_pythons_do() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let cases = golden["options"].as_array().unwrap();
    assert!(cases.len() >= 20);
    let text = |v: &Value| match v {
        Value::Null => Value::Null,
        Value::String(_) => v.clone(),
        Value::Bool(b) => Value::String(if *b { "True" } else { "False" }.into()),
        other => Value::String(herdr_inbox::pyjson::dumps(other)),
    };
    for case in cases {
        let got = normalize_options(&case["input"]);
        match case.get("error").and_then(Value::as_str) {
            Some("KeyError") => assert_eq!(got, Err(OptionsError::NoLabel), "{case}"),
            Some(_) => assert_eq!(got, Err(OptionsError::NotOptions), "{case}"),
            None => {
                let want = case["output"].as_array().map(|list| {
                    list.iter()
                        .map(|o| json!({"label": o["label"], "description": text(&o["description"]), "preview": text(&o["preview"])}))
                        .collect::<Vec<_>>()
                });
                let got = got.unwrap().map(|list| {
                    list.iter()
                        .map(|o| json!({"label": o.label, "description": o.description, "preview": o.preview}))
                        .collect::<Vec<_>>()
                });
                assert_eq!(got, want, "{case}");
            }
        }
    }
}

/// #79 review L3: an escaped lone surrogate in a stored JSON column (Python's
/// json.dumps writes one for a lone surrogate in a str) does not fail the read:
/// Python keeps the surrogate, a Rust string holds U+FFFD there.
#[test]
fn a_lone_surrogate_in_a_json_column_reads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), std::time::Duration::from_secs(5)).unwrap();
    let new = NewQuestion {
        question: "Odd?",
        pane_id: Some("w1:p1"),
        ..NewQuestion::default()
    };
    let q = store.insert(&new).unwrap();
    store
        .db()
        .execute(
            "UPDATE question_ext SET answer_json = ?, options_json = ? WHERE id = ?",
            rusqlite::params![
                r#"{"text": "a\ud83d"}"#,
                r#"[{"label": "\udc00", "description": null, "preview": null}]"#,
                q.id
            ],
        )
        .unwrap();
    let got = store.get(q.id).unwrap().unwrap();
    assert_eq!(got.answer_json, Some(json!({"text": "a\u{FFFD}"})));
    assert_eq!(got.options.unwrap()[0].label, "\u{FFFD}");
    let owner = Owner {
        session_id: None,
        pane_id: Some("w1:p1"),
    };
    assert_eq!(
        store.mine(owner, true, &Fake::default()).unwrap().len(),
        1,
        "the whole list still reads"
    );
}

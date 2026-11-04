//! The remote side where the CLI golden cannot show it: what Python fails
//! on with a traceback (the golden captures only what Python replies), and
//! the record and the action failing together.

use std::time::{Duration, SystemTime};

use serde_json::{Map, Value, json};

use super::apply::{After, apply};
use super::export::export;
use super::keys::{db_id, qkey};
use crate::store::Store;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, pane_id, question, norm, status) \
             VALUES (1, '2001-01-01T00:00:00+00:00', 'w1:p1', 'Merge?', 'merge?', 'open')",
        )
        .unwrap();
    (dir, store)
}

/// A request for question 1 with the right ids.
fn req(store: &Store, rid: &str, action: &str, detail: Value) -> Map<String, Value> {
    let q = store.get(1).unwrap().unwrap();
    let dbid = db_id(store.db()).unwrap();
    let key = qkey(&dbid, &q);
    let v = json!({"request_id": rid, "db_id": dbid, "id": 1, "qkey": key, "action": action, "detail": detail});
    v.as_object().unwrap().clone()
}

fn recorded(store: &Store, rid: &str) -> Option<(Option<i64>, String, Option<String>)> {
    store
        .db()
        .query_row(
            "SELECT qid, outcome, reason FROM applied_requests WHERE request_id = ?",
            [rid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok()
}

#[test]
fn an_answer_python_raises_on_is_refused_and_recorded() {
    let (_dir, store) = store();
    for (n, detail) in [
        json!({"text": 5}),
        json!({"choices": 5}),
        json!({"choices": true}),
        json!({"choices": [1]}),
        json!({"choices": ["a", null]}),
        json!({"choices": ["a"], "notes": ["a"]}),
        json!({"choices": ["a"], "notes": "a: b"}),
        json!({"choices": ["a"], "notes": {"a": 3}}),
    ]
    .into_iter()
    .enumerate()
    {
        let rid = format!("m{n}");
        let (reply, after) = apply(&store, &req(&store, &rid, "answer", detail.clone())).unwrap();
        assert_eq!(
            reply,
            json!({"outcome": "rejected", "reason": "bad answer"}),
            "{detail}"
        );
        assert_eq!(after, None);
        assert_eq!(recorded(&store, &rid).unwrap().1, "rejected", "{detail}");
    }
    // what Python reads without fault: a falsy text, a note on an unpicked label
    let ok = json!({"text": 0, "choices": ["a"], "notes": {"b": 3, "a": ""}});
    let (reply, after) = apply(&store, &req(&store, "fine", "answer", ok)).unwrap();
    assert_eq!(reply, json!({"outcome": "applied"}));
    assert_eq!(
        after,
        Some(After::Deliver {
            qid: 1,
            force: false,
            pane: Some("w1:p1".into())
        })
    );
    assert_eq!(store.get(1).unwrap().unwrap().answer.as_deref(), Some("a"));
}

#[test]
fn ids_python_cannot_bind_are_recorded_as_null() {
    let (_dir, store) = store();
    for (rid, id, want) in [
        ("l", json!([1]), None),
        ("o", json!({"id": 1}), None),
        ("big", json!(u64::MAX), None),
        ("text", json!("7"), Some(7)), // INTEGER affinity, as for Python's bind
        ("real", json!(7.0), Some(7)),
    ] {
        let mut r = req(&store, rid, "dismiss", Value::Null);
        r.insert("id".into(), id);
        let (reply, _) = apply(&store, &r).unwrap();
        assert_eq!(
            reply,
            json!({"outcome": "rejected", "reason": "not that question"}),
            "{rid}"
        );
        assert_eq!(recorded(&store, rid).unwrap().0, want, "{rid}");
    }
    assert_eq!(store.get(1).unwrap().unwrap().status, "open");
}

#[test]
fn the_record_and_the_action_commit_together() {
    let (_dir, store) = store();
    store
        .db()
        .execute_batch(
            "CREATE TEMP TRIGGER refuse BEFORE INSERT ON applied_requests \
             BEGIN SELECT RAISE(ABORT, 'disk full'); END",
        )
        .unwrap();
    let r = req(&store, "r1", "answer", json!({"text": "yes"}));
    assert!(apply(&store, &r).is_err());
    let q = store.get(1).unwrap().unwrap();
    assert_eq!(
        (q.status.as_str(), q.answer),
        ("open", None),
        "the answer went with the record"
    );
    store.db().execute_batch("DROP TRIGGER refuse").unwrap();
    assert_eq!(apply(&store, &r).unwrap().0, json!({"outcome": "applied"}));
    assert_eq!(
        apply(&store, &r).unwrap().0,
        json!({"outcome": "already_applied"})
    );
}

#[test]
fn export_makes_no_data_writes_but_its_id() {
    let (_dir, store) = store();
    let changes = |s: &Store| -> i64 {
        s.db()
            .query_row("SELECT COUNT(*) FROM changes", [], |r| r.get(0))
            .unwrap()
    };
    let before = changes(&store);
    let out = export(&store, &Map::new(), SystemTime::now()).unwrap();
    assert_eq!(out["ids"], json!([1]));
    assert_eq!(out["db_id"], json!(db_id(store.db()).unwrap()));
    assert_eq!(changes(&store), before);
}

use std::time::{Duration, UNIX_EPOCH};

use inbox_core::popup::Detail;
use serde_json::json;

use super::*;
use crate::store::questions::NewQuestion;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    (dir, s)
}

fn ask(s: &Store, pane: &str, group: Option<i64>) -> i64 {
    let q = NewQuestion {
        question: "q?",
        options: None,
        context: None,
        asked_by: None,
        pane_id: Some(pane),
        tab_id: None,
        tab_label: None,
        session_id: None,
        cwd: None,
        header: None,
        multi_select: false,
        group_id: group,
    };
    s.insert(&q).unwrap().id
}

fn alert(s: &Store, status: &str) -> i64 {
    s.db()
        .execute(
            "INSERT INTO alerts (created_at, kind, severity, title, pane_id, status) \
             VALUES ('2026-11-04T00:00:00+00:00', 'agent', 'info', 't', 'w1:p1', ?)",
            [status],
        )
        .unwrap();
    s.db().last_insert_rowid()
}

#[test]
fn a_read_takes_the_open_rows_and_whatever_the_popup_shows_whole() {
    let (_d, s) = store();
    let q1 = ask(&s, "w1:p1", None);
    let q2 = ask(&s, "w1:p1", Some(q1));
    let q3 = ask(&s, "w1:p2", None);
    s.set_status(q2, "dismissed", None).unwrap();
    s.set_status(q3, "withdrawn", None).unwrap();
    let a1 = alert(&s, "open");
    let a2 = alert(&s, "acked");

    let fresh = read(&s, &Shown::default(), UNIX_EPOCH).unwrap();
    let ids: Vec<i64> = fresh.questions.iter().map(|q| q.id).collect();
    assert_eq!((ids, fresh.open.clone()), (vec![q1], vec![q1]));
    assert_eq!(
        (fresh.alerts.len(), fresh.open_alerts.clone()),
        (1, vec![a1])
    );

    let mut shown = Shown::default();
    shown.groups.insert(q1);
    shown.groups.insert(q3);
    shown.alerts.insert(a2);
    let snap = read(&s, &shown, UNIX_EPOCH).unwrap();
    let ids: Vec<(i64, &str)> = snap
        .questions
        .iter()
        .map(|q| (q.id, q.status.as_str()))
        .collect();
    assert_eq!(ids, [(q1, "open"), (q2, "dismissed"), (q3, "withdrawn")]);
    assert_eq!(snap.open, [q1]);
    let alerts: Vec<(i64, &str)> = snap
        .alerts
        .iter()
        .map(|a| (a.id, a.status.as_str()))
        .collect();
    assert_eq!(alerts, [(a1, "open"), (a2, "acked")]);
    assert_eq!(snap.open_alerts, [a1]);
}

#[test]
fn a_draft_reads_back_as_the_detail_it_was_stored_from() {
    let d = Detail {
        choices: vec!["b".into(), "a".into()],
        notes: vec![("b".into(), "x".into()), ("a".into(), "y".into())],
        text: Some("t".into()),
    };
    let v = convert::detail_json(&d);
    assert_eq!(
        crate::pyjson::dumps(&v),
        r#"{"choices": ["b", "a"], "notes": {"b": "x", "a": "y"}, "text": "t"}"#
    );
    assert_eq!(convert::detail(&v), Some(d));
    assert_eq!(
        convert::detail(&json!({})),
        None,
        "Python: an empty draft is no draft"
    );
    assert_eq!(convert::detail(&serde_json::Value::Null), None);
}

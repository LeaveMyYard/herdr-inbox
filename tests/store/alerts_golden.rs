//! Alerts replay tests/fixtures/store-alerts.json: Python's `alerts.py` over
//! `STORE_ALERT_OPS` (agent alerts, the blocked-prompt scan with herdr stubbed
//! at its function boundary), with `alerts` and `blocked_panes` after each
//! step and what Python did to herdr (`effects`). The Rust store, with the
//! same inputs, must leave the same rows, return the same results and ask for
//! the same effects.

use std::time::Duration;

use herdr_inbox::store::Store;
use herdr_inbox::store::alerts::{
    Alert, Asker, BlockedScan, Effect, RaiseOutcome, SeenAgent, WithdrawOutcome,
};
use herdr_inbox::store::questions::{NewQuestion, Owner};
use serde_json::{Value, json};

use crate::common::migrated;
use crate::family::Fake;
use crate::golden::{deep_mask, i, question, s, sql, tables};

const GOLDEN: &str = include_str!("../fixtures/store-alerts.json");

fn alert(a: &Alert) -> Value {
    deep_mask(json!({
        "id": a.id, "created_at": a.created_at, "kind": a.kind, "severity": a.severity, "title": a.title,
        "message": a.message, "pane_id": a.pane_id, "tab_id": a.tab_id, "tab_label": a.tab_label,
        "session_id": a.session_id, "asked_by": a.asked_by, "herdr_server": a.herdr_server, "status": a.status,
        "acked_at": a.acked_at, "closed_at": a.closed_at, "reported_at": a.reported_at,
    }))
}

fn alerts(list: Vec<Alert>) -> Value {
    Value::Array(list.iter().map(alert).collect())
}

fn effects(list: &[Effect]) -> Value {
    Value::Array(
        list.iter()
            .map(|e| match e {
                Effect::Notify { title, body } => json!(["notify", title, body]),
                Effect::Markers(pane) => json!(["markers", pane]),
            })
            .collect(),
    )
}

fn asker(c: &Value) -> Asker<'_> {
    Asker {
        owner: Owner {
            session_id: s(c, "session_id"),
            pane_id: s(c, "pane_id"),
        },
        tab_id: s(c, "tab_id"),
        tab_label: s(c, "tab_label"),
    }
}

/// One step: its result and the effects it asked for.
fn run(store: &Store, scan: &mut BlockedScan, op: &str, a: &Value) -> (Value, Value) {
    let files = Fake::default();
    let none = || json!([]);
    let server = s(a, "server").unwrap_or("/srv/a");
    match op {
        "sql" => {
            sql(store, a);
            (Value::Null, none())
        }
        "raise_alert" => {
            let got = store
                .raise_alert(
                    asker(&a["caller"]),
                    s(a, "title").unwrap(),
                    s(a, "message"),
                    s(a, "severity").unwrap(),
                    s(a, "asked_by"),
                    server,
                    &files,
                )
                .unwrap();
            match got {
                RaiseOutcome::Raised(id, eff) => {
                    (json!({"id": id, "status": "open"}), effects(&eff))
                }
                RaiseOutcome::AlreadyOpen(id) => {
                    (json!({"id": id, "status": "already_open"}), none())
                }
                RaiseOutcome::NoTitle => (json!({"status": "error"}), none()),
                RaiseOutcome::Unavailable => (json!({"status": "unavailable"}), none()),
            }
        }
        "withdraw" => {
            let aid = i(a, "aid");
            match store
                .withdraw_alert(asker(&a["caller"]).owner, aid, &files)
                .unwrap()
            {
                WithdrawOutcome::Withdrawn(eff) => {
                    (json!({"id": aid, "status": "withdrawn"}), effects(&eff))
                }
                WithdrawOutcome::NotYours | WithdrawOutcome::NotOpen(_) => {
                    (json!({"id": aid, "status": "error"}), none())
                }
            }
        }
        "acknowledge" => match store.acknowledge_alert(i(a, "aid")).unwrap() {
            Some(eff) => (json!(true), effects(&eff)),
            None => (json!(false), none()),
        },
        "acknowledged_unreported" => (
            alerts(
                store
                    .acknowledged_unreported(asker(&a["caller"]).owner, &files)
                    .unwrap(),
            ),
            none(),
        ),
        "mine" => (
            alerts(store.my_alerts(asker(&a["caller"]).owner, &files).unwrap()),
            none(),
        ),
        "get" => (
            store
                .alert(i(a, "aid"))
                .unwrap()
                .as_ref()
                .map_or(Value::Null, alert),
            none(),
        ),
        "open_alerts" => (alerts(store.open_alerts().unwrap()), none()),
        "scan" | "scan_failed" => {
            let seen: Option<Vec<SeenAgent>> = a.get("agents").map(|list| {
                list.as_array()
                    .unwrap()
                    .iter()
                    .filter_map(SeenAgent::from_json)
                    .collect()
            });
            let labels = a.get("labels").cloned().unwrap_or(json!({}));
            let t = a.get("t").and_then(Value::as_f64).unwrap_or(0.0);
            let (n, eff) = scan
                .scan(store, server, seen.as_deref(), t, |tab| {
                    labels.get(tab).and_then(Value::as_str).map(String::from)
                })
                .unwrap();
            (json!(n), effects(&eff))
        }
        "insert" => {
            let new = NewQuestion {
                question: s(a, "question").unwrap(),
                session_id: s(a, "session_id"),
                pane_id: s(a, "pane_id"),
                ..NewQuestion::default()
            };
            (question(&store.insert(&new).unwrap()), none())
        }
        "set_answer" => (
            json!(
                store
                    .set_answer(i(a, "qid"), s(a, "answer").unwrap(), None)
                    .unwrap()
            ),
            none(),
        ),
        "set_status" => (
            json!(
                store
                    .set_status(i(a, "qid"), s(a, "status").unwrap(), s(a, "note"))
                    .unwrap()
            ),
            none(),
        ),
        "acknowledge_answers" => {
            let ids: Vec<i64> = serde_json::from_value(a["qids"].clone()).unwrap();
            store.acknowledge(&ids).unwrap();
            (Value::Null, none())
        }
        "sessions_in_play" => {
            let mut got = store.sessions_in_play().unwrap();
            got.sort();
            (json!(got), none())
        }
        other => panic!("no Rust call for {other}"),
    }
}

#[test]
fn every_alert_step_leaves_pythons_rows_and_asks_for_pythons_effects() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    let mut scan = BlockedScan::default();
    let steps = golden["steps"].as_array().unwrap();
    assert!(steps.len() >= 40);
    for (n, step) in steps.iter().enumerate() {
        let op = step["op"].as_str().unwrap();
        let (result, eff) = run(&store, &mut scan, op, &step["args"]);
        assert_eq!(result, step["result"], "step {n} {op}: result");
        assert_eq!(eff, step["effects"], "step {n} {op}: effects");
        assert_eq!(
            tables(&store, &["alerts", "blocked_panes"]),
            step["tables"],
            "step {n} {op}: tables"
        );
    }
}

/// #83 review M1 #22: a prompt first seen exactly [`BLOCKED_AFTER`] ago alerts
/// (Python skips only while `now - since < 15`). The step is retried if the
/// clock second changed in between, so `since` is exactly the cutoff.
#[test]
fn a_prompt_alerts_at_exactly_fifteen_seconds() {
    use herdr_inbox::store::alerts::BLOCKED_AFTER;
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
        let before = herdr_inbox::clock::now();
        let since = herdr_inbox::clock::now_minus(BLOCKED_AFTER);
        store
            .db()
            .execute("INSERT INTO blocked_panes (herdr_server, pane_id, since) VALUES ('/srv/a', 'w1:p5', ?)", [since])
            .unwrap();
        let agent = SeenAgent {
            pane_id: "w1:p5".into(),
            status: Some("blocked".into()),
            tab_id: None,
            session: None,
            agent: None,
        };
        let (_, eff) = BlockedScan::default()
            .scan(&store, "/srv/a", Some(&[agent]), 0.0, |_| None)
            .unwrap();
        if herdr_inbox::clock::now() == before {
            assert!(
                matches!(eff.first(), Some(Effect::Notify { .. })),
                "alerted at the edge: {eff:?}"
            );
            return;
        }
    }
    panic!("the clock second changed on every try");
}

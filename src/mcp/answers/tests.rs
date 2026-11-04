use std::time::Duration;

use serde_json::json;

use super::check_answers;
use crate::claude::Transcripts;
use crate::mcp::caller::Caller;
use crate::store::Store;
use crate::store::alerts::Asker;
use crate::store::questions::{NewQuestion, Owner};

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    (dir, s)
}

fn caller(session: Option<&str>) -> Caller {
    Caller {
        pane_id: Some("w1:p1".into()),
        session_id: session.map(str::to_owned),
        ..Caller::default()
    }
}

fn filed(s: &Store, session: Option<&str>, q: &str) -> i64 {
    s.insert(&NewQuestion {
        question: q,
        pane_id: Some("w1:p1"),
        session_id: session,
        ..NewQuestion::default()
    })
    .unwrap()
    .id
}

fn check(s: &Store, c: &Caller) -> serde_json::Value {
    check_answers(s, c, &Transcripts::new(None)).unwrap()
}

#[test]
fn unseen_answers_come_once_and_are_marked_seen() {
    let (_d, s) = store();
    let a = filed(&s, Some(S), "Deploy?");
    let b = filed(&s, Some(S), "Open?");
    s.set_answer(a, "no", None).unwrap();
    let c = caller(Some(S));
    assert_eq!(
        check(&s, &c),
        json!({"answers": [{"id": a, "question": "Deploy?", "answer": "no"}]})
    );
    assert_eq!(check(&s, &c), json!({"answers": []}));
    assert!(s.get(a).unwrap().unwrap().acknowledged_at.is_some());
    assert!(s.get(b).unwrap().unwrap().acknowledged_at.is_none());
}

#[test]
fn only_the_callers_answers() {
    let (_d, s) = store();
    let mine = filed(&s, Some(S), "mine?");
    let theirs = filed(&s, Some("bbbbbbbb-0000-4000-8000-00000000000b"), "theirs?");
    let paneless = filed(&s, None, "session-less?");
    for q in [mine, theirs, paneless] {
        s.set_answer(q, "x", None).unwrap();
    }
    let ids = |v: serde_json::Value| -> Vec<i64> {
        v["answers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_i64().unwrap())
            .collect()
    };
    assert_eq!(ids(check(&s, &caller(Some(S)))), [mine]);
    // no session: the pane's session-less questions only
    assert_eq!(ids(check(&s, &caller(None))), [paneless]);
    // neither: nothing
    assert_eq!(check(&s, &Caller::default()), json!({"answers": []}));
    assert!(s.get(theirs).unwrap().unwrap().acknowledged_at.is_none());
}

#[test]
fn an_answer_the_agent_fetched_is_never_claimed_for_typing() {
    let (_d, s) = store();
    let q = filed(&s, Some(S), "One?");
    s.set_answer(q, "yes", None).unwrap();
    check(&s, &caller(Some(S)));
    assert_eq!(s.claim_delivery(q).unwrap(), None);
}

#[test]
fn an_answer_in_flight_is_returned_and_its_claim_kept_as_in_python() {
    let (_d, s) = store();
    let q = filed(&s, Some(S), "One?");
    s.set_answer(q, "yes", None).unwrap();
    let claim = s.claim_delivery(q).unwrap().expect("claimed");
    assert_eq!(check(&s, &caller(Some(S)))["answers"][0]["id"], json!(q));
    let after = s.get(q).unwrap().unwrap();
    assert_eq!(after.delivery_error.as_deref(), Some(claim.0.as_str()));
    assert!(after.acknowledged_at.is_some());
}

#[test]
fn a_stale_delivery_error_on_a_fetched_answer_is_dropped() {
    let (_d, s) = store();
    let q = filed(&s, Some(S), "One?");
    s.set_answer(q, "yes", None).unwrap();
    s.record_delivery(q, Some("no live agent in pane"), false, None, false)
        .unwrap();
    check(&s, &caller(Some(S)));
    assert_eq!(s.get(q).unwrap().unwrap().delivery_error, None);
}

#[test]
fn acknowledged_alerts_are_reported_once() {
    let (_d, s) = store();
    let files = Transcripts::new(None);
    let owner = Owner {
        session_id: Some(S),
        pane_id: Some("w1:p1"),
    };
    let asker = Asker {
        owner,
        tab_id: None,
        tab_label: None,
    };
    let raised = s
        .raise_alert(asker, "Look", None, "warning", None, "/sock", &files)
        .unwrap();
    let crate::store::alerts::RaiseOutcome::Raised(aid, _) = raised else {
        panic!("{raised:?}")
    };
    let c = caller(Some(S));
    assert_eq!(check(&s, &c), json!({"answers": []}));
    s.acknowledge_alert(aid).unwrap();
    assert_eq!(
        check(&s, &c),
        json!({"answers": [], "alerts_acknowledged": [{"id": format!("A{aid}"), "title": "Look"}]})
    );
    assert_eq!(check(&s, &c), json!({"answers": []}));
}

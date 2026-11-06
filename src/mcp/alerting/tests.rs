use std::time::Duration;

use serde_json::json;

use super::*;
use crate::store::hosts::{CORE_SERVER, CoreSeat};

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn caller(session: &str) -> Caller {
    Caller {
        pane_id: Some("w1:p1".into()),
        session_id: Some(session.into()),
        ..Caller::default()
    }
}

/// Another session's alert (e.g. after /clear) is not the caller's: it stays open.
#[test]
fn another_sessions_alert_cannot_be_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    let files = Transcripts::new(None);
    let raise = Raise {
        title: "Question Title 9001",
        message: None,
        severity: "info",
        asked_by: None,
    };
    let (r, fx) = alert_operator(&s, &caller(S), &raise, "/sock", &files).unwrap();
    let aid = r["id"].as_i64().unwrap();
    assert_eq!(fx.len(), 2); // a notification and the markers
    let other = caller("bbbbbbbb-0000-4000-8000-00000000000b");
    let (r, fx) = withdraw_alert(&s, &other, aid, &files).unwrap();
    assert_eq!(
        r,
        json!({"id": aid, "status": "error", "message": format!("A{aid} is not an alert of this session.")})
    );
    assert!(fx.is_empty());
    assert_eq!(s.alert(aid).unwrap().unwrap().status, "open");
    let (r, fx) = withdraw_alert(&s, &caller(S), aid, &files).unwrap();
    assert_eq!(r["status"], json!("withdrawn"));
    assert_eq!(fx, [Effect::Markers(Some("w1:p1".into()))]);
}

#[test]
fn a_raised_alert_notifies_counts_in_the_badge_and_dedupes() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    let files = Transcripts::new(None);
    let c = Caller {
        tab_id: Some("w1:t1".into()),
        tab_label: Some("tab-A".into()),
        ..caller(S)
    };
    let raise = Raise {
        title: "Question Title 123",
        message: Some("Question Text 345"),
        severity: "urgent",
        asked_by: Some("deployer"),
    };
    let (r, fx) = alert_operator(&s, &c, &raise, "/sock", &files).unwrap();
    assert_eq!((&r["status"], &r["id"]), (&json!("open"), &json!(1)));
    assert!(
        r["message"]
            .as_str()
            .unwrap()
            .contains("no answer comes back")
    );
    assert_eq!(
        fx,
        [
            Effect::Notify {
                title: "A1 · urgent · tab-A".into(),
                body: "Question Title 123".into()
            },
            Effect::Markers(Some("w1:p1".into()))
        ]
    );
    assert_eq!(s.open_counts().unwrap().get("w1:p1"), Some(&1));
    let new = crate::store::questions::NewQuestion {
        pane_id: Some("w1:p1"),
        question: "Question Title 456",
        ..Default::default()
    };
    s.insert(&new).unwrap();
    assert_eq!(s.open_counts().unwrap().get("w1:p1"), Some(&2));
    let (r, fx) = alert_operator(&s, &c, &raise, "/sock", &files).unwrap();
    assert_eq!(r["status"], json!("already_open"));
    assert!(fx.is_empty());
}

#[test]
fn a_core_agent_raises_an_alert_with_a_seat_and_nothing_goes_to_herdr() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    let files = Transcripts::new(None);
    let c = Caller {
        pane_id: None,
        core: Some(CoreSeat {
            terminal: "t3".into(),
            machine: None,
        }),
        ..caller(S)
    };
    let raise = Raise {
        title: "Question Title 123",
        message: None,
        severity: "info",
        asked_by: None,
    };
    let (r, fx) = alert_operator(&s, &c, &raise, "/sock", &files).unwrap();
    assert_eq!(r["status"], json!("open"));
    assert!(fx.is_empty());
    let a = s.alert(1).unwrap().unwrap();
    assert_eq!(a.pane_id, None);
    assert_eq!(a.herdr_server.as_deref(), Some(CORE_SERVER));
    let terminal: String = s
        .db()
        .query_row("SELECT terminal FROM alert_host WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(terminal, "t3");
    let (again, _) = alert_operator(&s, &c, &raise, "/sock", &files).unwrap();
    assert_eq!(again["status"], json!("already_open"));
    let (r, fx) = withdraw_alert(&s, &c, 1, &files).unwrap();
    assert_eq!(r["status"], json!("withdrawn"));
    assert_eq!(fx, [Effect::Markers(None)]);
}

#[test]
fn neither_a_pane_nor_a_seat_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    let c = Caller {
        pane_id: None,
        ..caller(S)
    };
    let raise = Raise {
        title: "Title",
        message: None,
        severity: "info",
        asked_by: None,
    };
    let (r, _) = alert_operator(&s, &c, &raise, "/sock", &Transcripts::new(None)).unwrap();
    assert_eq!(r, crate::mcp::receipts::unavailable());
    assert!(s.alert(1).unwrap().is_none());
}

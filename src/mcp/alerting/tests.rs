use std::time::Duration;

use serde_json::json;

use super::*;

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

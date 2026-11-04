use std::time::Duration;

use serde_json::json;

use super::*;
use crate::store::questions::NewQuestion;

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    (dir, s)
}

fn caller(session: &str) -> Caller {
    Caller {
        pane_id: Some("w1:p1".into()),
        session_id: Some(session.into()),
        ..Caller::default()
    }
}

fn filed(s: &Store, session: &str, q: &str) -> i64 {
    let q = NewQuestion {
        question: q,
        pane_id: Some("w1:p1"),
        session_id: Some(session),
        ..NewQuestion::default()
    };
    s.insert(&q).unwrap().id
}

#[test]
fn another_sessions_question_cannot_be_withdrawn_or_listed() {
    let (_d, s) = store();
    let files = Transcripts::new(None);
    let q = filed(&s, S, "Which bucket?");
    let other = caller("bbbbbbbb-0000-4000-8000-00000000000b"); // e.g. after /clear
    let (r, fx) = withdraw_question(&s, &other, q, "x", &files).unwrap();
    assert_eq!(r["status"], json!("error"));
    assert!(fx.is_empty());
    assert_eq!(s.get(q).unwrap().unwrap().status, "open");
    assert_eq!(
        list_my_questions(&s, &other, true, &files).unwrap(),
        json!({"questions": []})
    );
    // its own: withdrawn, the reason kept, the pane's markers refreshed
    let (r, fx) = withdraw_question(&s, &caller(S), q, "found it in terraform", &files).unwrap();
    assert_eq!(
        r,
        json!({"id": q, "status": "withdrawn", "message": format!("Q{q} withdrawn.")})
    );
    assert_eq!(fx, [Effect::Markers(Some("w1:p1".into()))]);
    let got = s.get(q).unwrap().unwrap();
    assert_eq!(
        (got.status.as_str(), got.answer.as_deref()),
        ("withdrawn", Some("found it in terraform"))
    );
}

#[test]
fn an_answer_with_detail_lists_it() {
    let (_d, s) = store();
    let q = filed(&s, S, "DB?");
    let detail = json!({"choices": ["pg"], "notes": {"pg": "v16"}});
    s.set_answer(q, "pg (v16)", Some(&detail)).unwrap();
    let got = list_my_questions(&s, &caller(S), false, &Transcripts::new(None)).unwrap();
    assert_eq!(got["questions"][0]["answer_detail"], detail);
    assert_eq!(got["questions"][0]["answer"], json!("pg (v16)"));
}

/// review #118 L3: Python `if self.answer_json`: an empty detail is left out.
#[test]
fn an_empty_answer_detail_is_left_out() {
    let (_d, s) = store();
    let q = filed(&s, S, "DB?");
    s.set_answer(q, "pg", None).unwrap();
    s.db()
        .execute(
            "UPDATE question_ext SET answer_json = '{}' WHERE id = ?",
            [q],
        )
        .unwrap();
    let got = list_my_questions(&s, &caller(S), false, &Transcripts::new(None)).unwrap();
    assert_eq!(
        got["questions"][0],
        json!({"id": q, "status": "answered", "question": "DB?", "answer": "pg"})
    );
}

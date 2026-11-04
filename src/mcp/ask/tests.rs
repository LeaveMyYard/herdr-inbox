use serde_json::json;

use super::checks::items;
use crate::mcp::args::{Ask, Item};

#[test]
fn an_empty_question_or_list_counts_as_none() {
    let refused =
        json!({"status": "error", "message": "Pass either `question` or `questions` (not both)."});
    let empty = Ask {
        question: Some(String::new()),
        questions: Some(Vec::new()),
        ..Ask::default()
    };
    assert_eq!(items(&empty), Err(refused.clone()));
    let one_and_empty_list = Ask {
        question: Some("q?".into()),
        questions: Some(Vec::new()),
        ..Ask::default()
    };
    assert_eq!(items(&one_and_empty_list).unwrap()[0].question, "q?");
}

#[test]
fn one_question_carries_the_calls_options_header_and_multi_select() {
    let a = Ask {
        question: Some("q?".into()),
        options: Some(vec![json!("a")]),
        header: Some("H".into()),
        multi_select: true,
        ..Ask::default()
    };
    assert_eq!(
        items(&a),
        Ok(vec![Item {
            question: "q?".into(),
            header: Some("H".into()),
            options: Some(vec![json!("a")]),
            multi_select: true
        }])
    );
}

#[test]
fn the_option_limit_applies_to_every_item() {
    let item = |n: usize| Item {
        question: "q".into(),
        header: None,
        options: Some((0..n).map(|i| json!(i.to_string())).collect()),
        multi_select: false,
    };
    let a = Ask {
        questions: Some(vec![item(9), item(10)]),
        ..Ask::default()
    };
    assert_eq!(
        items(&a),
        Err(
            json!({"status": "error", "message": "At most 9 options; ask fewer or split the question."})
        )
    );
    let ok = Ask {
        questions: Some(vec![item(9); 4]),
        ..Ask::default()
    };
    assert_eq!(items(&ok).unwrap().len(), 4);
}

// -- filing (Python `service.ask_group`) ---------------------------------------------------

use std::time::Duration;

use super::file::{Call, ask_group, single};
use crate::claude::Transcripts;
use crate::mcp::caller::Caller;
use crate::store::Store;
use crate::store::alerts::Effect;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    (dir, s)
}

fn caller() -> Caller {
    Caller {
        pane_id: Some("w1:p1".into()),
        tab_id: Some("w1:t1".into()),
        tab_label: Some("my-tab".into()),
        session_id: Some("aaaaaaaa-0000-4000-8000-00000000000a".into()),
        cwd: Some("/r".into()),
    }
}

fn item(q: &str) -> Item {
    Item {
        question: q.into(),
        header: None,
        options: None,
        multi_select: false,
    }
}

fn ask(
    s: &Store,
    c: &Caller,
    items: &[Item],
    title: Option<&str>,
) -> (serde_json::Value, Vec<Effect>) {
    let call = Call {
        items,
        context: Some("ctx"),
        asked_by: Some("release agent"),
        title,
    };
    ask_group(s, c, &call, "/sock", &Transcripts::new(None)).unwrap()
}

#[test]
fn a_new_question_is_filed_for_its_caller_on_this_server() {
    let (_d, s) = store();
    let mut it = item("  Deploy to prod?\n");
    it.header = Some("  A very long header text ".into());
    it.options = Some(vec![
        json!("yes"),
        json!({"label": "no", "description": "wait"}),
    ]);
    let (r, fx) = ask(&s, &caller(), &[it], None);
    let r = single(r);
    assert_eq!(r["id"], json!(1));
    assert_eq!(r["status"], json!("open"));
    assert!(
        r["message"]
            .as_str()
            .unwrap()
            .starts_with("Recorded as Q1. ")
    );
    let q = s.get(1).unwrap().unwrap();
    assert_eq!(q.question, "Deploy to prod?");
    assert_eq!(q.header.as_deref(), Some("A very long ")); // stripped, 12 characters
    assert_eq!(
        (
            q.pane_id.as_deref(),
            q.tab_label.as_deref(),
            q.session_id.as_deref(),
            q.cwd.as_deref()
        ),
        (
            Some("w1:p1"),
            Some("my-tab"),
            Some("aaaaaaaa-0000-4000-8000-00000000000a"),
            Some("/r")
        )
    );
    assert_eq!(
        (q.context.as_deref(), q.asked_by.as_deref()),
        (Some("ctx"), Some("release agent"))
    );
    assert_eq!(s.server_of(1).unwrap().as_deref(), Some("/sock"));
    assert_eq!(
        fx,
        [
            Effect::Notify {
                title: "Q1 · my-tab".into(),
                body: "A very long : Deploy to prod?".into()
            },
            Effect::Markers(Some("w1:p1".into()))
        ]
    );
}

#[test]
fn a_repeat_gets_the_open_receipt_or_the_answer() {
    let (_d, s) = store();
    let c = caller();
    ask(&s, &c, &[item("Deploy to prod?")], None);
    let (r, fx) = ask(&s, &c, &[item("deploy to PROD")], None);
    assert_eq!(
        single(r),
        json!({"id": 1, "status": "already_open", "message": "Q1 is already with the operator. Do not ask again; keep working."})
    );
    assert!(fx.is_empty());
    assert_eq!(s.get(1).unwrap().unwrap().repeat_count, 1);
    s.set_answer(1, "no, wait for Monday", None).unwrap();
    let (r, _) = ask(&s, &c, &[item("Deploy to prod?")], None);
    assert_eq!(
        single(r),
        json!({"id": 1, "status": "answered", "answer": "no, wait for Monday",
               "message": "Q1 was already answered by the operator: no, wait for Monday"})
    );
    assert!(s.get(1).unwrap().unwrap().acknowledged_at.is_some()); // the repeat has the answer now
    // another conversation (after /clear) asks anew
    let mut other = c.clone();
    other.session_id = Some("bbbbbbbb-0000-4000-8000-00000000000b".into());
    assert_eq!(
        single(ask(&s, &other, &[item("Deploy to prod?")], None).0)["id"],
        json!(2)
    );
}

#[test]
fn a_group_shares_its_id_and_title_and_dedupes_within_the_call() {
    let (_d, s) = store();
    let (r, fx) = ask(
        &s,
        &caller(),
        &[item("q1?"), item("q2?"), item("Q1?")],
        Some("Rollout settings"),
    );
    assert_eq!(r["group"], json!(1));
    let statuses: Vec<_> = r["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["status"].clone())
        .collect();
    assert_eq!(
        statuses,
        [json!("open"), json!("open"), json!("already_open")]
    );
    assert_eq!(
        r["message"],
        json!(
            "Filed Q1 (open), Q2 (open), Q1 (already_open). Answers arrive later as `[inbox]` messages naming the Q ids; keep working and do not ask these again. If an answer seems overdue, call check_answers."
        )
    );
    assert_eq!(s.get(2).unwrap().unwrap().group(), 1);
    assert_eq!(
        s.get(2).unwrap().unwrap().group_title.as_deref(),
        Some("Rollout settings")
    );
    assert_eq!(
        fx[0],
        Effect::Notify {
            title: "Q1, Q2 · my-tab".into(),
            body: "Rollout settings".into()
        }
    );
    // a lone question takes the call's title too
    let (r, _) = ask(&s, &caller(), &[item("Lone?")], Some("Lone call"));
    let lone = single(r)["id"].as_i64().unwrap();
    assert_eq!(
        s.get(lone).unwrap().unwrap().group_title.as_deref(),
        Some("Lone call")
    );
    // all repeats: no group, nothing to herdr
    let (r, fx) = ask(&s, &caller(), &[item("q1?")], None);
    assert_eq!((r["group"].clone(), fx.len()), (json!(null), 0));
}

#[test]
fn without_a_pane_nothing_is_filed() {
    let (_d, s) = store();
    let mut c = caller();
    c.pane_id = None;
    let (r, fx) = ask(&s, &c, &[item("q?")], None);
    assert_eq!(r, crate::mcp::receipts::unavailable());
    assert_eq!(single(r), crate::mcp::receipts::unavailable());
    assert!(fx.is_empty() && s.get(1).unwrap().is_none());
}

#[test]
fn the_notification_names_the_pane_when_the_tab_has_no_label() {
    let (_d, s) = store();
    let mut c = caller();
    c.tab_label = Some(String::new());
    let (_, fx) = ask(&s, &c, &[item("first line\nsecond")], None);
    assert_eq!(
        fx[0],
        Effect::Notify {
            title: "Q1 · w1:p1".into(),
            body: "first line".into()
        }
    );
}

#[test]
fn first_line_is_pythons() {
    use super::notify::first_line;
    assert_eq!(first_line("  \n a\u{2028}b", 80), "a");
    assert_eq!(first_line("", 80), "");
    assert_eq!(first_line("abcdef", 4), "abc…");
    assert_eq!(first_line("abcd", 4), "abcd");
}

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
use crate::store::hosts::{CORE_SERVER, CoreSeat};

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
        core: None,
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
    ask_from(s, c, items, title, None)
}

fn ask_from(
    s: &Store,
    c: &Caller,
    items: &[Item],
    title: Option<&str>,
    tool_use_id: Option<&str>,
) -> (serde_json::Value, Vec<Effect>) {
    let call = Call {
        items,
        context: Some("ctx"),
        asked_by: Some("release agent"),
        title,
        tool_use_id,
    };
    ask_group(s, c, &call, "/sock", &Transcripts::new(None)).unwrap()
}

#[test]
fn every_question_a_call_files_records_its_tool_use_id() {
    let (_d, s) = store();
    let items = [item("Question Text 345"), item("Question Text 346")];
    ask_from(&s, &caller(), &items, None, Some("toolu_01AbCdEf"));
    for id in [1, 2] {
        assert_eq!(
            s.tool_use_id(id).unwrap().as_deref(),
            Some("toolu_01AbCdEf"),
            "Q{id}"
        );
    }
    // a repeat files nothing, so it keeps the first call's id
    ask_from(&s, &caller(), &items[..1], None, Some("toolu_02Other"));
    assert_eq!(s.tool_use_id(1).unwrap().as_deref(), Some("toolu_01AbCdEf"));
    // a call without one, or with something that is no id, records none
    ask_from(&s, &caller(), &[item("Question Text 347")], None, None);
    ask_from(
        &s,
        &caller(),
        &[item("Question Text 348")],
        None,
        Some("Question Title 123"),
    );
    assert_eq!(s.tool_use_id(3).unwrap(), None);
    assert_eq!(s.tool_use_id(4).unwrap(), None);
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

/// Python `test_concurrent_identical_asks_create_one_question`: a parallel ask of the
/// same question commits while this one waits for the write lock; deduped in the same
/// transaction as the insert, this one finds it.
#[test]
fn a_parallel_identical_ask_files_one_question() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    Store::open_migrating(&path, Duration::from_secs(3)).unwrap();
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap(); // the other ask holds the write lock
    let p = path.clone();
    let asking = std::thread::spawn(move || {
        let s = Store::open(&p, Duration::from_secs(10)).unwrap();
        single(ask(&s, &caller(), &[item("Merge PR 7?")], None).0)
    });
    // the other ask files the same question and commits; this ask is waiting by now (if it
    // is not, the test passes without proving anything, never fails wrongly)
    std::thread::sleep(Duration::from_millis(300));
    let c = caller();
    other
        .execute(
            "INSERT INTO questions (created_at, pane_id, session_id, question, norm) \
             VALUES ('2026-11-04T00:00:00+00:00', ?, ?, 'Merge PR 7?', 'merge pr 7')",
            rusqlite::params![c.pane_id, c.session_id],
        )
        .unwrap();
    other
        .execute("INSERT INTO question_ext (id, group_id) VALUES (last_insert_rowid(), last_insert_rowid())", [])
        .unwrap();
    other.execute_batch("COMMIT").unwrap();
    let r = asking.join().unwrap();
    assert_eq!(r["status"], json!("already_open"), "{r}");
    let n: i64 = other
        .query_row("SELECT COUNT(*) FROM questions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn a_dismissed_question_can_be_asked_again() {
    let (_d, s) = store();
    let c = caller();
    let first = single(ask(&s, &c, &[item("Question Title 123")], None).0);
    assert!(s.set_status(1, "dismissed", None).unwrap());
    let again = single(ask(&s, &c, &[item("Question Title 123")], None).0);
    assert_eq!(again["status"], json!("open"));
    assert_ne!(again["id"], first["id"]);
}

#[test]
fn the_pane_is_the_fallback_owner_without_a_session() {
    let (_d, s) = store();
    let mut c = caller();
    c.session_id = None;
    let first = single(ask(&s, &c, &[item("Question Title 123")], None).0);
    let same_pane = single(ask(&s, &c, &[item("Question Title 123")], None).0);
    assert_eq!(same_pane["status"], json!("already_open"));
    assert_eq!(same_pane["id"], first["id"]);
    c.pane_id = Some("w1:p2".into());
    let other_pane = single(ask(&s, &c, &[item("Question Title 123")], None).0);
    assert_eq!(other_pane["status"], json!("open"));
}

fn core_caller() -> Caller {
    Caller {
        session_id: Some("aaaaaaaa-0000-4000-8000-00000000000a".into()),
        cwd: Some("/r".into()),
        core: Some(CoreSeat {
            terminal: "t3".into(),
            machine: None,
        }),
        ..Caller::default()
    }
}

#[test]
fn a_core_caller_is_filed_with_no_pane_and_a_seat() {
    let (_d, s) = store();
    let (r, fx) = ask(&s, &core_caller(), &[item("Question Title 123?")], None);
    let r = single(r);
    assert_eq!(r["status"], json!("open"));
    assert!(fx.is_empty(), "nothing is done to herdr for a core row");
    let q = s.get(1).unwrap().unwrap();
    assert_eq!((q.pane_id, q.tab_id, q.tab_label), (None, None, None));
    assert_eq!(s.server_of(1).unwrap().as_deref(), Some(CORE_SERVER));
    let (host, terminal): (String, String) = s
        .db()
        .query_row(
            "SELECT host, terminal FROM question_host WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((host.as_str(), terminal.as_str()), ("herdr-core", "t3"));
}

#[test]
fn a_core_receipt_promises_an_inbox_message_as_a_panes_does() {
    let (_d, s) = store();
    let (group, _) = ask(&s, &core_caller(), &[item("q1?"), item("q2?")], None);
    for message in [&group["message"], &group["questions"][0]["message"]] {
        let message = message.as_str().unwrap();
        assert!(message.contains("[inbox]"), "{message}");
        assert!(message.contains("If an answer seems overdue"), "{message}");
        assert!(!message.contains("herdr-app"), "{message}");
    }
}

#[test]
fn a_pane_caller_gets_no_seat() {
    let (_d, s) = store();
    ask(&s, &caller(), &[item("q?")], None);
    let seats: i64 = s
        .db()
        .query_row("SELECT COUNT(*) FROM question_host", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seats, 0);
    assert_eq!(s.server_of(1).unwrap().as_deref(), Some("/sock"));
}

#[test]
fn a_core_repeat_is_the_same_question() {
    let (_d, s) = store();
    ask(&s, &core_caller(), &[item("q?")], None);
    let (r, fx) = ask(&s, &core_caller(), &[item("q?")], None);
    assert_eq!(single(r)["status"], json!("already_open"));
    assert!(fx.is_empty());
}

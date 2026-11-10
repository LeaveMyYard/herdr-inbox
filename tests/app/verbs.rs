//! What a verb leaves in the database, beyond its result: which answer went to which question,
//! and that a stale token gets past nothing.

use std::time::Duration;

use herdr_inbox::app::verbs::reply::Reply;
use herdr_inbox::app::verbs::{Ctx, run};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::commands::{Started, live_claim};
use crate::world::{World, fixture};

fn world() -> World {
    World::of(&fixture("app-feed/every-kind.json"))
}

fn token(key: &str) -> String {
    let feed = fixture("app-feed/every-kind.json");
    let rows = feed["entries"].as_array().unwrap();
    let row = rows.iter().find(|e| e["key"] == key).unwrap();
    row["token"].as_str().unwrap().to_string()
}

fn verb(w: &World, words: &[&str]) -> (Reply, Started) {
    verb_on(w, words, true)
}

fn verb_on(w: &World, words: &[&str], core: bool) -> (Reply, Started) {
    let mut fx = Started::default();
    let argv: Vec<String> = words.iter().map(|s| (*s).to_string()).collect();
    let mut ctx = Ctx {
        db: &w.db,
        db_wait: Duration::from_millis(100),
        core_delivery: core,
        fx: &mut fx,
    };
    let reply = run(&argv, &mut ctx);
    (reply, fx)
}

fn out(reply: &Reply) -> Value {
    serde_json::from_str(&reply.stdout).unwrap()
}

fn answer(w: &World, id: i64) -> (String, Option<String>, Option<String>) {
    w.store
        .db()
        .query_row(
            "SELECT q.status, q.answer, e.answer_json FROM questions q JOIN question_ext e ON e.id = q.id WHERE q.id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn a_lone_answer_goes_to_that_question_with_its_parts_as_the_popup_stores_them() {
    let w = world();
    let t = token("group:12");
    let detail = r#"{"choices":["Option A"],"notes":{"Option A":"  Note Text 1 ","Option B":"gone"},"text":" Answer Text 567 "}"#;
    let (reply, _) = verb(
        &w,
        &[
            "answer",
            "--entry",
            "group:12",
            "--token",
            &t,
            "--question",
            "12",
            "--detail",
            detail,
            "--json",
        ],
    );
    assert_eq!(reply.exit, 0, "{reply:?}");
    let (status, text, parts) = answer(&w, 12);
    assert_eq!(status, "answered");
    let parts: Value = serde_json::from_str(&parts.unwrap()).unwrap();
    let want = json!({"choices": ["Option A"], "notes": {"Option A": "Note Text 1"}, "text": "Answer Text 567"});
    assert_eq!(parts, want);
    assert_eq!(
        text.unwrap(),
        herdr_inbox::service::answer::answer_text(&want)
    );
    assert_eq!(
        w.store.draft_text(12).unwrap(),
        None,
        "the draft went with it"
    );
}

#[test]
fn submit_gives_each_question_its_own_draft_and_leaves_the_rest_open() {
    let w = world();
    let t = token("group:14");
    let (reply, fx) = verb(
        &w,
        &[
            "answer",
            "--entry",
            "group:14",
            "--token",
            &t,
            "--question",
            "15",
            "--question",
            "14",
            "--question",
            "16",
            "--json",
        ],
    );
    assert_eq!(reply.exit, 0, "{reply:?}");
    assert_eq!(out(&reply)["done"], json!([15, 14]));
    assert_eq!(out(&reply)["skipped"], json!([16]));
    assert_eq!(answer(&w, 15).1.as_deref(), Some("Answer Text 567"));
    let first = answer(&w, 14).1.unwrap();
    assert!(
        first.contains("Option A") && first.contains("Option C"),
        "{first}"
    );
    assert_eq!(answer(&w, 16).0, "open");
    assert_eq!(fx.senders[0].0, ["deliver", "15", "14"]);
}

/// A check outside the transaction would read the old row and say `stale` while another
/// process holds the lock: inside it, the verb waits for the lock before it looks.
#[test]
fn the_token_is_checked_under_the_write_lock() {
    let w = world();
    let other = Connection::open(&w.db).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let wrong = "0".repeat(32);
    let (reply, _) = verb(
        &w,
        &["ack", "--entry", "alert:5", "--token", &wrong, "--json"],
    );
    assert_eq!(out(&reply)["code"], "busy", "{reply:?}");
    other.execute_batch("ROLLBACK").unwrap();
}

/// Everything else is checked with the same token: a stale one writes nothing anywhere.
#[test]
fn a_stale_token_writes_nothing_on_any_row() {
    let w = world();
    let wrong = "0".repeat(32);
    let detail = r#"{"choices":["Option A"],"notes":{},"text":null}"#;
    let tries: [&[&str]; 7] = [
        &[
            "answer",
            "--entry",
            "group:12",
            "--token",
            &wrong,
            "--question",
            "12",
            "--detail",
            detail,
            "--json",
        ],
        &[
            "answer",
            "--entry",
            "group:14",
            "--token",
            &wrong,
            "--question",
            "14",
            "--json",
        ],
        &[
            "draft",
            "--entry",
            "group:12",
            "--token",
            &wrong,
            "--question",
            "12",
            "--detail",
            detail,
            "--json",
        ],
        &[
            "dismiss", "--entry", "group:12", "--token", &wrong, "--json",
        ],
        &["retry", "--entry", "failed:8", "--token", &wrong, "--json"],
        &[
            "stop-showing",
            "--entry",
            "failed:8",
            "--token",
            &wrong,
            "--json",
        ],
        &["ack", "--entry", "alert:5", "--token", &wrong, "--json"],
    ];
    for t in tries {
        let (reply, fx) = verb(&w, t);
        assert_eq!(reply.exit, 1, "{t:?}");
        assert_eq!(out(&reply)["code"], "stale", "{t:?}");
        assert!(fx.order.is_empty(), "{t:?}: nothing starts");
    }
    assert_eq!(answer(&w, 12).0, "open");
    assert_eq!(w.store.draft_text(12).unwrap(), None);
    assert_eq!(w.store.retry_count(8).unwrap(), 0);
    assert!(!w.store.hidden(8).unwrap());
    assert_eq!(w.store.alert(5).unwrap().unwrap().status, "open");
}

/// With the popup and an overlay open on one group, the second one is told.
#[test]
fn the_second_of_two_uis_gets_stale_answered() {
    let w = world();
    let t = token("group:12");
    let detail = r#"{"choices":["Option A"],"notes":{},"text":null}"#;
    let words = [
        "answer",
        "--entry",
        "group:12",
        "--token",
        &t,
        "--question",
        "12",
        "--detail",
        detail,
        "--json",
    ];
    assert_eq!(verb(&w, &words).0.exit, 0);
    let (again, fx) = verb(&w, &words);
    assert_eq!(out(&again)["code"], "stale");
    assert_eq!(out(&again)["closed"], "answered");
    assert!(fx.senders.is_empty(), "no second send");
}

#[test]
fn a_named_question_that_closed_says_why_though_the_group_stays() {
    let w = world();
    w.store
        .db()
        .execute(
            "UPDATE questions SET status = 'withdrawn' WHERE id = 15",
            [],
        )
        .unwrap();
    let t = token("group:14");
    let (reply, _) = verb(
        &w,
        &[
            "answer",
            "--entry",
            "group:14",
            "--token",
            &t,
            "--question",
            "15",
            "--json",
        ],
    );
    assert_eq!(out(&reply)["code"], "stale");
    assert_eq!(out(&reply)["closed"], "withdrawn");
}

#[test]
fn a_repeated_draft_changes_nothing_and_says_so() {
    let w = world();
    let detail = r#"{"choices":["Option A"],"notes":{},"text":null}"#;
    let t = token("group:14");
    let words = [
        "draft",
        "--entry",
        "group:14",
        "--token",
        &t,
        "--question",
        "16",
        "--detail",
        detail,
        "--json",
    ];
    let (first, _) = verb(&w, &words);
    assert_eq!(out(&first)["done"], json!([16]));
    let moved = out(&first)["token"].as_str().unwrap().to_string();
    assert_ne!(moved, t);
    let at: Option<String> = w
        .store
        .db()
        .query_row(
            "SELECT drafted_at FROM question_ext WHERE id = 16",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let again = [
        "draft",
        "--entry",
        "group:14",
        "--token",
        &moved,
        "--question",
        "16",
        "--detail",
        detail,
        "--json",
    ];
    let (second, _) = verb(&w, &again);
    assert_eq!(second.exit, 0);
    assert_eq!(out(&second)["done"], json!([]));
    assert_eq!(out(&second)["token"].as_str().unwrap(), moved);
    let now: Option<String> = w
        .store
        .db()
        .query_row(
            "SELECT drafted_at FROM question_ext WHERE id = 16",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(at, now);
    let clear = [
        "draft",
        "--entry",
        "group:14",
        "--token",
        &moved,
        "--question",
        "16",
        "--clear",
        "--json",
    ];
    assert_eq!(out(&verb(&w, &clear).0)["done"], json!([16]));
}

/// A live claim keeps the row out of the feed but it is still a row: `retry` reads
/// `already_sending`, before the token, so a stale token here says what is really going on.
#[test]
fn a_live_claim_reads_as_already_sending_before_the_token() {
    let w = world();
    w.store
        .db()
        .execute(
            "UPDATE questions SET delivery_error = ? WHERE id = 8",
            params![live_claim()],
        )
        .unwrap();
    let wrong = "0".repeat(32);
    let (reply, _) = verb(
        &w,
        &["retry", "--entry", "failed:8", "--token", &wrong, "--json"],
    );
    assert_eq!(out(&reply)["code"], "already_sending");
    assert_eq!(w.store.retry_count(8).unwrap(), 0);
}

/// `retry` takes the host from the database: a pane row is forced, a core row never.
#[test]
fn retry_is_forced_on_a_pane_and_never_on_a_core_row() {
    let w = world();
    let t = token("failed:8");
    let (_, fx) = verb(
        &w,
        &["retry", "--entry", "failed:8", "--token", &t, "--json"],
    );
    assert_eq!(fx.senders[0].0, ["deliver", "8", "--force"]);
    let t = token("failed:9");
    let (reply, fx) = verb(
        &w,
        &["retry", "--entry", "failed:9", "--token", &t, "--json"],
    );
    assert_eq!(fx.senders[0].0, ["deliver", "9"], "{reply:?}");
    assert_eq!(fx.senders[0].1, None, "a core sender sets no socket");
}

/// The counter moved though nothing could be sent: a replay stays refused once the host is back.
#[test]
fn a_held_retry_is_done_and_its_token_moved() {
    let w = world();
    let t = token("failed:9");
    let words = ["retry", "--entry", "failed:9", "--token", &t, "--json"];
    let (reply, fx) = verb_on(&w, &words, false);
    assert_eq!(out(&reply)["delivery"], "held");
    assert!(fx.order.is_empty(), "core before B3: nothing starts");
    assert_eq!(w.store.retry_count(9).unwrap(), 1);
    assert_eq!(out(&verb_on(&w, &words, true).0)["code"], "stale");
}

/// Stored rows can hold NULL here: a row from before the column.
#[test]
fn a_null_counter_reads_as_zero() {
    let w = world();
    w.store
        .db()
        .execute(
            "UPDATE question_ext SET retry_count = NULL WHERE id = 8",
            [],
        )
        .unwrap();
    let t = token("failed:8");
    assert_eq!(
        verb(
            &w,
            &["retry", "--entry", "failed:8", "--token", &t, "--json"]
        )
        .0
        .exit,
        0
    );
    assert_eq!(w.store.retry_count(8).unwrap(), 1);
}

#[test]
fn a_draft_that_cannot_be_submitted_refuses_the_whole_verb() {
    let w = world();
    w.store
        .db()
        .execute(
            "UPDATE question_ext SET draft_json = '[1]' WHERE id = 15",
            [],
        )
        .unwrap();
    let group = w.store.group(14, false).unwrap();
    let t = herdr_inbox::app::feed::group::entry(
        &w.store,
        14,
        &group,
        herdr_inbox::app::feed::text::Text::Cut,
    )
    .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let (reply, fx) = verb(
        &w,
        &[
            "answer",
            "--entry",
            "group:14",
            "--token",
            &t,
            "--question",
            "14",
            "--question",
            "15",
            "--json",
        ],
    );
    assert_eq!(out(&reply)["code"], "error", "{reply:?}");
    assert_eq!(answer(&w, 14).0, "open", "the first one stays open too");
    assert!(fx.order.is_empty());
}

/// A choice that is no label can be anything: text in the wrong field, say.
#[test]
fn no_message_repeats_any_part_of_the_detail() {
    let words = "Answer Text 567";
    let t = token("group:12");
    for detail in [
        json!({"choices": [words], "notes": {}, "text": null}),
        json!({"choices": ["Option A", words], "notes": {}, "text": null}),
        json!({"choices": ["Option A", "Option A"], "notes": {words: words}, "text": words}),
    ] {
        let w = world();
        let detail = detail.to_string();
        let (reply, _) = verb(
            &w,
            &[
                "answer",
                "--entry",
                "group:12",
                "--token",
                &t,
                "--question",
                "12",
                "--detail",
                &detail,
                "--json",
            ],
        );
        assert_eq!(reply.exit, 1, "{detail}");
        assert!(!reply.stdout.contains(words), "{}", reply.stdout);
        assert!(reply.stderr.is_empty());
    }
}

mod show {
    use herdr_inbox::app::verbs::show::MAX_OUTPUT;

    use super::*;

    #[test]
    fn a_terminal_gets_the_question_its_context_and_its_options() {
        let w = world();
        let (reply, _) = verb(&w, &["show", "--question", "14"]);
        assert_eq!(reply.exit, 0, "{reply:?}");
        for part in [
            "Q14: Question Text 345",
            "Context Text 678",
            "- Option A",
            "Q15:",
        ] {
            assert!(reply.stdout.contains(part), "{part}: {}", reply.stdout);
        }
    }

    #[test]
    fn a_closed_question_in_a_terminal_says_so_on_stderr() {
        let w = world();
        w.store
            .db()
            .execute(
                "UPDATE questions SET status = 'dismissed' WHERE id = 12",
                [],
            )
            .unwrap();
        let (reply, _) = verb(&w, &["show", "--question", "12"]);
        assert_eq!((reply.exit, reply.stdout.as_str()), (1, ""));
        assert!(!reply.stderr.is_empty());
    }

    #[test]
    fn a_question_that_never_was_is_stale_removed() {
        let w = world();
        let (reply, _) = verb(&w, &["show", "--question", "999", "--json"]);
        assert_eq!(out(&reply)["closed"], "removed");
    }

    /// Absence from the feed is no closure: show reads the database, not the file.
    #[test]
    fn the_token_it_gives_is_taken_by_the_other_verbs() {
        let w = world();
        let (shown, _) = verb(&w, &["show", "--question", "12", "--json"]);
        let t = out(&shown)["row"]["token"].as_str().unwrap().to_string();
        assert_eq!(t, token("group:12"));
        let (reply, _) = verb(
            &w,
            &["dismiss", "--entry", "group:12", "--token", &t, "--json"],
        );
        assert_eq!(reply.exit, 0);
    }

    /// `exec` caps output: an oversize row must be a refusal, not a failure of unknown effect.
    #[test]
    fn a_row_past_what_exec_reads_is_a_refusal_that_names_the_terminal_form() {
        let w = world();
        w.store
            .db()
            .execute(
                "UPDATE questions SET question = ? WHERE id = 12",
                params!["x".repeat(MAX_OUTPUT + 1)],
            )
            .unwrap();
        let (reply, _) = verb(&w, &["show", "--question", "12", "--json"]);
        assert_eq!(reply.exit, 1);
        assert_eq!(out(&reply)["code"], "error");
        assert!(
            out(&reply)["message"]
                .as_str()
                .unwrap()
                .contains("terminal")
        );
        assert_eq!(
            verb(&w, &["show", "--question", "12"]).0.exit,
            0,
            "the terminal form has no such cap"
        );
    }

    #[test]
    fn it_writes_nothing() {
        let w = world();
        let version = |db: &Connection| -> i64 {
            db.query_row("PRAGMA data_version", [], |r| r.get(0))
                .unwrap()
        };
        let watcher = Connection::open(&w.db).unwrap();
        let before = version(&watcher);
        verb(&w, &["show", "--question", "14", "--json"]);
        assert_eq!(version(&watcher), before);
    }
}

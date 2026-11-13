use std::time::{Duration, UNIX_EPOCH};

use serde_json::json;

use super::{read_around, unavailable};
use crate::store::Store;
use crate::store::questions::NewQuestion;

#[test]
fn a_feed_that_says_nothing_has_no_rows_and_says_why() {
    let wall = UNIX_EPOCH + Duration::from_secs(1767348000);
    let feed = unavailable(
        wall,
        "the inbox database is not migrated to this version yet",
    );
    let want = include_str!("../../../../tests/fixtures/app-feed/not-migrated.json");
    assert_eq!(
        feed,
        serde_json::from_str::<serde_json::Value>(want).unwrap()
    );
    assert_eq!(feed["ok"], json!(false));
}

/// A question filed after the rows were read must not raise the mark: the toast mod would take
/// the mark and never announce a row it has not seen.
#[test]
fn a_mark_never_runs_ahead_of_its_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let other = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let q = NewQuestion {
        question: "Question Text 345",
        ..NewQuestion::default()
    };
    store.insert(&q).unwrap();
    let wall = UNIX_EPOCH + Duration::from_secs(1767348000);
    let feed = read_around(&store, wall, || {
        other.insert(&q).unwrap();
    })
    .unwrap();
    assert_eq!(feed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(feed["new_since"]["question_id"], 1);
}

/// The core worker's listing is in what the feed calls `auto_retry` (the review of #146, L3).
#[test]
fn a_core_row_the_core_worker_retries_is_listed() {
    use crate::store::hosts::{CORE_SERVER, CoreSeat};
    let dir = tempfile::tempdir().unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            ..NewQuestion::default()
        })
        .unwrap();
    store.set_server(q.id, Some(CORE_SERVER)).unwrap();
    let seat = CoreSeat {
        terminal: "t3".into(),
        machine: None,
    };
    store.set_question_host(q.id, &seat).unwrap();
    store.set_answer(q.id, "yes", None).unwrap();
    store
        .db()
        .execute_batch(
            "UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00', \
             delivery_error = 'pending: waiting: cannot read the agent''s terminal (x)'",
        )
        .unwrap();
    assert!(super::retried(&store).unwrap().contains(&q.id));
}

fn fresh() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    (dir, store)
}

/// A delivered answer from pane w1:p1, given a minute before the wall clock below.
fn answer_one(store: &Store, asked_by: &str) -> i64 {
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            pane_id: Some("w1:p1"),
            asked_by: Some(asked_by),
            ..NewQuestion::default()
        })
        .unwrap();
    store.set_answer(q.id, "Option B", None).unwrap();
    store
        .db()
        .execute(
            "UPDATE questions SET answered_at = '2026-01-02T09:59:00+00:00', \
             delivered_at = '2026-01-02T09:59:00+00:00' WHERE id = ?",
            [q.id],
        )
        .unwrap();
    q.id
}

#[test]
fn the_feed_publishes_what_was_answered_beside_the_entries() {
    let (_d, store) = fresh();
    let id = answer_one(&store, "Agent 1");
    let wall = UNIX_EPOCH + Duration::from_secs(1767348000);
    let feed = super::read(&store, wall).unwrap();
    assert_eq!(feed["v"], json!(1), "still v1: the additions are additive");
    assert_eq!(feed["minor"], json!(super::MINOR));
    assert_eq!(
        feed["entries"],
        json!([]),
        "an answer is no entry: v1 readers see an empty inbox"
    );
    assert_eq!(feed["answered"][0]["key"], json!(format!("answered:{id}")));
    assert_eq!(feed["answered_omitted"], json!(0));
}

/// History is fitted after every entry: an open question is never left out for it.
#[test]
fn answered_rows_never_crowd_out_an_open_question() {
    let (_d, store) = fresh();
    // about 40 KiB each, never cut: 30 of them do not fit beside the open group
    let big = "x".repeat(40 * 1024);
    for _ in 0..30 {
        answer_one(&store, &big);
    }
    // an open group of about 900 KiB, in `asked_by`, which is never cut either
    let open = "y".repeat(900 * 1024);
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            pane_id: Some("w1:p2"),
            asked_by: Some(&open),
            ..NewQuestion::default()
        })
        .unwrap();
    let wall = UNIX_EPOCH + Duration::from_secs(1767348000);
    let feed = super::read(&store, wall).unwrap();
    assert_eq!(
        feed["entries"].as_array().unwrap().len(),
        1,
        "the open group is in"
    );
    assert_eq!(feed["entries"][0]["group"], json!(q.id));
    assert_eq!(feed["omitted"]["groups"], json!(0));
    let kept = feed["answered"].as_array().unwrap().len();
    let left = feed["answered_omitted"].as_u64().unwrap() as usize;
    assert!(kept > 0 && left > 0, "kept {kept}, left out {left}");
    assert_eq!(kept + left, 20, "20 per asker, all from w1:p1");
    assert!(serde_json::to_vec(&feed).unwrap().len() <= super::MAX_FEED);
    assert!(
        feed["errors"].as_array().unwrap().is_empty(),
        "history left out is no incomplete inbox"
    );
}

#[test]
fn a_group_carries_its_calls_tool_use_id_outside_its_token() {
    let (_d, store) = fresh();
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            pane_id: Some("w1:p1"),
            ..NewQuestion::default()
        })
        .unwrap();
    let wall = UNIX_EPOCH + Duration::from_secs(1767348000);
    let before = super::read(&store, wall).unwrap()["entries"][0].clone();
    assert_eq!(before["tool_use_id"], json!(null));
    store.set_tool_use_id(q.id, "toolu_01AbCdEf").unwrap();
    let after = super::read(&store, wall).unwrap()["entries"][0].clone();
    assert_eq!(after["tool_use_id"], json!("toolu_01AbCdEf"));
    assert_eq!(
        after["token"], before["token"],
        "v1's token fields are frozen"
    );
}

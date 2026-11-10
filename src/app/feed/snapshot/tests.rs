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

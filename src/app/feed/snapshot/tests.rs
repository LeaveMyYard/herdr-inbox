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

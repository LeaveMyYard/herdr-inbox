use std::time::Duration;

use serde_json::Value;

use super::read;
use crate::store::Store;
use crate::store::questions::NewQuestion;

/// A row and its token come from one moment: a question closing between the two reads does not
/// make a row out of half of each.
#[test]
fn the_row_comes_from_one_moment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let other = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            ..NewQuestion::default()
        })
        .unwrap();
    let reply = read(&store, q.id, || {
        assert!(other.set_status(q.id, "dismissed", None).unwrap());
    })
    .unwrap();
    let value: Value = serde_json::from_str(&reply.stdout).unwrap();
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["row"]["questions"][0]["id"], q.id);
}

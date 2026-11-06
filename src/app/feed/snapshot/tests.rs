use std::time::{Duration, UNIX_EPOCH};

use serde_json::json;

use super::unavailable;

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

use serde_json::json;

use super::{Item, MAX_IDS, Need, fit, left_out};

fn row(key: &str, size: usize) -> serde_json::Value {
    json!({"key": key, "pad": "x".repeat(size)})
}

fn item(key: &str, size: usize, need: Need) -> Item {
    Item {
        entry: row(key, size),
        need,
    }
}

fn keys(rows: &[serde_json::Value]) -> Vec<&str> {
    rows.iter().map(|r| r["key"].as_str().unwrap()).collect()
}

/// A huge row takes nothing behind it with it, and what fits stays in display order.
#[test]
fn a_row_too_big_is_skipped_and_the_walk_goes_on() {
    let items = vec![
        item("alert:1", 10, Need::Alert),
        item("group:2", 5000, Need::Group(vec![2, 3])),
        item("group:4", 10, Need::Group(vec![4])),
        item("failed:5", 10, Need::Failed),
        item("remote:6", 10, Need::Remote),
    ];
    let (kept, omitted) = fit(items, 1000);
    assert_eq!(
        keys(&kept),
        ["alert:1", "group:4", "failed:5", "remote:6"],
        "{omitted}"
    );
    assert_eq!(omitted["groups"], 1);
    assert_eq!(omitted["questions"], 2);
    assert_eq!(omitted["question_ids"], json!([2, 3]));
    assert_eq!(left_out(&omitted), 1);
}

/// Questions come first, whatever the display order says.
#[test]
fn room_goes_by_need_not_by_display_order() {
    let items = vec![
        item("alert:1", 300, Need::Alert),
        item("group:9", 300, Need::Group(vec![9])),
        item("group:2", 300, Need::Group(vec![2])),
        item("failed:5", 300, Need::Failed),
        item("remote:6", 300, Need::Remote),
    ];
    let (kept, omitted) = fit(items, 700);
    assert_eq!(keys(&kept), ["group:9", "group:2"]);
    assert_eq!(omitted["alerts"], 1);
    assert_eq!(omitted["failed"], 1);
    assert_eq!(omitted["remote"], 1);
    let items = vec![
        item("group:9", 300, Need::Group(vec![9])),
        item("group:2", 300, Need::Group(vec![2])),
    ];
    let (kept, omitted) = fit(items, 400);
    assert_eq!(keys(&kept), ["group:2"], "oldest first");
    assert_eq!(omitted["question_ids"], json!([9]));
}

#[test]
fn past_the_cap_the_count_goes_on_and_the_ids_stop_at_the_oldest() {
    let ids: Vec<i64> = (1..=1500).rev().collect();
    let (kept, omitted) = fit(vec![item("group:1", 100, Need::Group(ids))], 10);
    assert!(kept.is_empty());
    assert_eq!(omitted["questions"], 1500);
    let listed = omitted["question_ids"].as_array().unwrap();
    assert_eq!(listed.len(), MAX_IDS);
    assert_eq!(listed[0], 1);
    assert_eq!(listed[MAX_IDS - 1], 1000);
}

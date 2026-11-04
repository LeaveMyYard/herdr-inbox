use std::time::Duration;

use super::*;

#[test]
fn a_local_target_never_asks_for_the_machines() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open_migrating(&d.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, pane_id, question, norm, status) VALUES \
             (4, 'x', 'w1:p1', 'a', 'a', 'open'), (2, 'x', 'w1:p1', 'b', 'b', 'open')",
        )
        .unwrap();
    let ask = |kind: &str, value: &str| {
        let r = Request {
            kind: kind.into(),
            value: value.into(),
            hold: true,
        };
        resolve(&store, &r, || panic!("machines asked")).unwrap()
    };
    assert_eq!(ask("question", "q4"), Some((Target::Group(4), Some(4))));
    assert_eq!(
        ask("pane", "w1:p1"),
        Some((Target::Group(2), None)),
        "the oldest group"
    );
    assert_eq!(ask("group", " 2 "), Some((Target::Group(2), None)));
    assert_eq!(ask("group", "3"), None);
}

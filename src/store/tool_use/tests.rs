use std::time::Duration;

use super::{MAX_LEN, checked};
use crate::store::Store;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    (dir, s)
}

#[test]
fn a_tool_use_id_is_kept_and_read_back() {
    let (_dir, s) = store();
    s.db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, question, norm) VALUES (1, 't', 'Question Text 345', 'n'); \
             INSERT INTO question_ext (id, group_id) VALUES (1, 1)",
        )
        .unwrap();
    assert_eq!(s.tool_use_id(1).unwrap(), None);
    s.set_tool_use_id(1, "toolu_01AbCdEf").unwrap();
    assert_eq!(s.tool_use_id(1).unwrap().as_deref(), Some("toolu_01AbCdEf"));
    assert_eq!(s.tool_use_id(2).unwrap(), None, "no row");
}

#[test]
fn only_an_id_shaped_value_is_kept() {
    assert_eq!(checked(Some("toolu_01AbCd-Ef")), Some("toolu_01AbCd-Ef"));
    let longest = "a".repeat(MAX_LEN);
    assert_eq!(checked(Some(&longest)), Some(longest.as_str()));
    let too_long = "a".repeat(MAX_LEN + 1);
    for bad in [
        "",
        too_long.as_str(),
        "toolu 01",
        "toolu_01\n",
        "Question Text 345!",
        "toolu_é",
    ] {
        assert_eq!(checked(Some(bad)), None, "{bad:?}");
    }
    assert_eq!(checked(None), None);
}

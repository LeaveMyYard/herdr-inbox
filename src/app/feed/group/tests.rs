use std::time::Duration;

use serde_json::json;

use super::entry;
use crate::app::feed::text::{LONG, Text};
use crate::store::Store;
use crate::store::questions::NewQuestion;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    let store = Store::open_migrating(&db, Duration::from_secs(5)).unwrap();
    (dir, store)
}

#[test]
fn a_whole_row_keeps_the_cut_rows_token_so_the_verbs_take_it() {
    let (_dir, store) = store();
    let text = "x".repeat(LONG + 10);
    let q = store
        .insert(&NewQuestion {
            question: &text,
            pane_id: Some("w1:p1"),
            ..NewQuestion::default()
        })
        .unwrap();
    let open = store.group(q.id, false).unwrap();
    let cut = entry(&store, q.id, &open, Text::Cut).unwrap();
    let whole = entry(&store, q.id, &open, Text::Whole).unwrap();
    assert_eq!(cut["questions"][0]["cut"], json!(true));
    assert_eq!(
        cut["questions"][0]["question"].as_str().unwrap().len(),
        LONG
    );
    assert_eq!(whole["questions"][0]["cut"], json!(false));
    assert_eq!(whole["questions"][0]["question"].as_str().unwrap(), text);
    assert_eq!(whole["token"], cut["token"]);
}

#[test]
fn a_draft_the_popup_takes_for_none_is_null() {
    let (_dir, store) = store();
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            ..NewQuestion::default()
        })
        .unwrap();
    store
        .db()
        .execute(
            "UPDATE question_ext SET draft_json = '{}' WHERE id = ?",
            [q.id],
        )
        .unwrap();
    let open = store.group(q.id, false).unwrap();
    let row = entry(&store, q.id, &open, Text::Cut).unwrap();
    assert_eq!(row["questions"][0]["draft"], json!(null));
}

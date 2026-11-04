use std::time::Duration;

use herdr_agent_kit::herdr_api::ServerId;

use super::*;
use crate::store::Store;

/// final pass #110 L1: a tab the inbox decorated with its configured prefix (`Q`) is
/// stripped of it, read from the store's `tab_decor`, not only of the default `❓`.
#[test]
fn a_tabs_own_marks_come_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    Store::open_migrating(&db, Duration::from_secs(3)).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO tab_decor (tab_id, mark) VALUES ('w1:t1', 'Q'), ('w1:t1', '')",
        [],
    )
    .unwrap();
    let live = Live {
        herdr: Herdr::new(ServerId::new(dir.path().join("no-herdr.sock"))),
        files: Transcripts::new(None),
        state_dir: dir.path().to_path_buf(),
        db: db.clone(),
    };
    let mut marks = live.marks(Some("w1:t1")).unwrap();
    marks.sort();
    assert_eq!(marks, ["Q", "❓"]);
    assert_eq!(live.marks(Some("w1:t2")).unwrap(), ["❓"]);
}

use std::time::Duration;

use rusqlite::params;

use super::{CORE, CoreSeat};
use crate::store::Store;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    (dir, s)
}

fn row(s: &Store, table: &str, id: i64) -> (String, Option<String>, Option<String>) {
    s.db()
        .query_row(
            &format!("SELECT host, terminal, machine FROM {table} WHERE id = ?"),
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn a_seat_is_recorded_beside_its_question_and_its_alert() {
    let (_dir, s) = store();
    let seat = CoreSeat {
        terminal: "t3".into(),
        machine: None,
    };
    s.set_question_host(7, &seat).unwrap();
    s.set_alert_host(9, &seat).unwrap();
    for (table, id) in [("question_host", 7), ("alert_host", 9)] {
        assert_eq!(row(&s, table, id), (CORE.into(), Some("t3".into()), None));
    }
}

#[test]
fn a_row_has_one_seat() {
    let (_dir, s) = store();
    let seat = CoreSeat {
        terminal: "t3".into(),
        machine: None,
    };
    s.set_question_host(7, &seat).unwrap();
    assert!(s.set_question_host(7, &seat).is_err());
}

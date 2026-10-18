use herdr_inbox::store::readiness::{expected, missing};
use herdr_inbox::store::revision::{MIGRATION_REVISION, REVISION_BASE};
use herdr_inbox::store::{OpenError, Store};
use rusqlite::Connection;

use crate::common::*;

#[test]
fn the_captured_schema_is_this_revisions() {
    assert_eq!(expected().revision, MIGRATION_REVISION);
    assert!(expected().tables["questions"].contains("delivery_error"));
    assert!(expected().others.contains("questions_v6"));
}

#[test]
fn a_stamped_database_is_used_without_any_check() {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "user_version", REVISION_BASE + MIGRATION_REVISION)
        .unwrap();
    assert!(
        missing(&db).unwrap().is_empty(),
        "this revision: not checked"
    );
    db.pragma_update(None, "user_version", REVISION_BASE + MIGRATION_REVISION + 1)
        .unwrap();
    assert!(
        missing(&db).unwrap().is_empty(),
        "a higher revision: not checked"
    );
}

#[test]
fn any_other_stamp_is_checked_against_the_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let db = open(&path);
    for stamp in [0, 1_668_105_336, REVISION_BASE] {
        db.pragma_update(None, "user_version", stamp).unwrap();
        assert!(
            missing(&db).unwrap().is_empty(),
            "complete schema, stamp {stamp}"
        );
    }
    db.execute_batch("DROP VIEW questions_v6; DROP INDEX alerts_status")
        .unwrap();
    assert_eq!(missing(&db).unwrap(), ["alerts_status", "questions_v6"]);
    let empty = Connection::open_in_memory().unwrap();
    let all = missing(&empty).unwrap();
    assert_eq!(all[0], "alerts.acked_at", "tables first, sorted");
    assert!(all.contains(&"sessions.hook_seen_at".to_owned()));
    assert_eq!(
        all.last().map(String::as_str),
        Some("questions_v6"),
        "then the other objects, sorted"
    );
}

#[test]
fn a_hook_store_never_opens_an_older_schema_and_never_writes_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let old = open(&path);
    old.execute_batch("CREATE TABLE questions (id INTEGER PRIMARY KEY, question TEXT)")
        .unwrap();
    old.pragma_update(None, "user_version", 1_668_105_336)
        .unwrap();
    drop(old);
    let before = std::fs::read(&path).unwrap();
    match Store::open(&path, std::time::Duration::from_millis(500)) {
        Err(OpenError::NotReady(missing)) => {
            assert!(missing.contains(&"sessions.session_id".to_owned()))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "no migration, no stamp"
    );
    let stamp: i64 = open(&path)
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamp, 1_668_105_336);
}

#[test]
fn a_hook_store_never_creates_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("none.db");
    assert!(matches!(
        Store::open(&path, std::time::Duration::ZERO),
        Err(OpenError::Sqlite(_))
    ));
    assert!(!path.exists());
    let store = Store::open(&migrated(dir.path()), std::time::Duration::ZERO).unwrap();
    let n: i64 = store
        .db()
        .query_row("SELECT COUNT(*) FROM questions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn a_hook_store_waits_its_busy_timeout() {
    // Python Store(timeout=RECORD_TIMEOUT): SQLite's own busy timeout governs its statements
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), std::time::Duration::from_millis(500)).unwrap();
    let ms: i64 = store
        .db()
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ms, 500);
}

#[test]
fn foreign_keys_are_not_enforced_as_in_python() {
    // Python's sqlite3 leaves foreign_keys off; the bundled SQLite defaults it on
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), std::time::Duration::ZERO).unwrap();
    let on: i64 = store
        .db()
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(on, 0);
    store
        .db()
        .execute(
            "INSERT INTO question_ext (id, hook_misses) VALUES (999, 1)",
            [],
        )
        .unwrap();
}

#[test]
fn a_hook_store_on_an_older_schema_asks_for_its_revision() {
    // #75 review M2: the Rust hooks write the same revision-wanted marker as Python's opens
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let old = open(&path);
    old.execute_batch("CREATE TABLE questions (id INTEGER PRIMARY KEY, question TEXT)")
        .unwrap();
    drop(old);
    assert!(matches!(
        Store::open(&path, std::time::Duration::ZERO),
        Err(OpenError::NotReady(_))
    ));
    let marker = std::fs::read_to_string(dir.path().join("old.db.revision-wanted")).unwrap();
    assert!(
        marker.starts_with(&format!(
            "{} ",
            herdr_inbox::store::revision::MIGRATION_REVISION
        )),
        "{marker}"
    );
}

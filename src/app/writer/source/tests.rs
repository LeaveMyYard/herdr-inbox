use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::Value;

use super::Source;
use crate::store::Store;
use crate::store::questions::NewQuestion;

const WALL: Duration = Duration::from_secs(1_767_348_000);

fn feed(source: &mut Source) -> Value {
    serde_json::from_slice(&source.feed(UNIX_EPOCH + WALL)).unwrap()
}

#[test]
fn a_missing_database_is_no_empty_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let mut source = Source::new(&path);
    let f = feed(&mut source);
    assert_eq!(f["ok"], false);
    assert_eq!(f["errors"][0], "the inbox database does not exist yet");
    assert!(!path.exists(), "the writer creates nothing");
}

#[test]
fn a_database_not_migrated_says_so_and_stays_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE questions (id INTEGER)")
        .unwrap();
    let mut source = Source::new(&path);
    let f = feed(&mut source);
    assert_eq!(f["ok"], false);
    assert_eq!(
        f["errors"][0],
        "the inbox database is not migrated to this version yet"
    );
    let stamp: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamp, 0, "the writer never migrates");
    assert!(no_marker(&path), "and asks no worker for one");
}

fn no_marker(db: &Path) -> bool {
    std::fs::read_dir(db.parent().unwrap())
        .unwrap()
        .all(|e| !e.unwrap().file_name().to_string_lossy().contains("wanted"))
}

#[test]
fn a_commit_by_anyone_else_shows_as_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let mut source = Source::new(&path);
    assert_eq!(feed(&mut source)["ok"], true);
    source.changed();
    assert!(!source.changed(), "nothing happened");
    let q = NewQuestion {
        question: "Question Text 345",
        ..NewQuestion::default()
    };
    store.insert(&q).unwrap();
    assert!(source.changed());
    assert_eq!(feed(&mut source)["entries"].as_array().unwrap().len(), 1);
}

/// A row that cannot be read must not read as an inbox without it.
#[test]
fn a_row_that_cannot_be_read_gives_no_feed_rather_than_a_short_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let q = NewQuestion {
        question: "Question Text 345",
        ..NewQuestion::default()
    };
    let q = store.insert(&q).unwrap();
    store
        .db()
        .execute(
            "UPDATE question_ext SET draft_json = 'not json' WHERE id = ?",
            [q.id],
        )
        .unwrap();
    let mut source = Source::new(&path);
    let f = feed(&mut source);
    assert_eq!(f["ok"], false);
    assert_eq!(f["entries"].as_array().unwrap().len(), 0);
    assert_eq!(f["errors"][0], "the inbox database could not be read");
}

#[test]
fn a_database_moved_away_stops_reading_as_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let mut source = Source::new(&path);
    assert_eq!(feed(&mut source)["ok"], true);
    drop(store);
    std::fs::rename(&path, dir.path().join("elsewhere.db")).unwrap();
    let f = feed(&mut source);
    assert_eq!(f["ok"], false);
    assert_eq!(f["errors"][0], "the inbox database does not exist yet");
}

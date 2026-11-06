//! What older and newer releases leave in the database stays readable
//! (Python tests/test_compat.py, tests/test_schema_migrations.py): the v0.1
//! `questions` table keeps its columns, a future column is ignored, older
//! views are left for older processes, and a pre-bridge stamp changes nothing
//! for a process that does not migrate.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use herdr_inbox::store::busy::is_busy;
use herdr_inbox::store::migrate::{MigrateError, apply_schema, migration};
use herdr_inbox::store::questions::NewQuestion;
use herdr_inbox::store::revision::STAMP;
use herdr_inbox::store::{OpenError, Store};
use rusqlite::Connection;

use crate::common::{migrated, open};

const TIMEOUT: Duration = Duration::from_secs(5);
/// The stamp of 0.10.3-0.10.5 (Python compat_prebridge.CURRENT_PRE_BRIDGE).
const PRE_BRIDGE: i64 = 1_668_105_336;
/// The `questions` table as 0.1 created it (tests/compat_v01_store.py).
const V01_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS questions (
    id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL, pane_id TEXT, tab_id TEXT,
    tab_label TEXT, session_id TEXT, cwd TEXT, asked_by TEXT, question TEXT NOT NULL, options TEXT,
    context TEXT, norm TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'open', answer TEXT,
    answered_at TEXT, delivered_at TEXT, delivery_error TEXT, acknowledged_at TEXT,
    repeat_count INTEGER NOT NULL DEFAULT 0, delivery_attempts INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX IF NOT EXISTS questions_status ON questions(status);
    CREATE INDEX IF NOT EXISTS questions_session ON questions(session_id);";

fn columns(db: &Connection, table: &str) -> Vec<String> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
    let names = stmt.query_map([], |r| r.get(1)).unwrap();
    names.map(Result::unwrap).collect()
}

fn stamp_of(path: &Path) -> i64 {
    open(path)
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn the_questions_table_keeps_the_v01_column_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    open(&path).execute_batch(V01_SCHEMA).unwrap();
    let before = columns(&open(&path), "questions");
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    assert_eq!(columns(&open(&path), "questions"), before);
}

#[test]
fn a_column_a_future_version_added_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), TIMEOUT).unwrap();
    store
        .db()
        .execute_batch("ALTER TABLE questions ADD COLUMN added_in_v03 TEXT")
        .unwrap();
    let new = NewQuestion {
        question: "Question Title 123",
        ..NewQuestion::default()
    };
    let id = store.insert(&new).unwrap().id;
    assert_eq!(
        store.get(id).unwrap().unwrap().question,
        "Question Title 123"
    );
}

#[test]
fn views_are_versioned_and_older_ones_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let db = open(&path);
    for statement in &migration().schema {
        db.execute_batch(statement).unwrap();
    }
    db.execute_batch("CREATE VIEW questions_v2 AS SELECT id FROM questions")
        .unwrap();
    drop(db);
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    let db = open(&path);
    let mut stmt = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'view'")
        .unwrap();
    let views: BTreeSet<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        views,
        BTreeSet::from(["questions_v2".into(), "questions_v6".into()])
    );
    let shown = columns(&db, "questions_v6");
    assert!(shown.iter().any(|c| c == "group_title") && shown.iter().any(|c| c == "draft_json"));
}

#[test]
fn a_non_migrating_open_on_a_stale_stamp_uses_the_db_without_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    open(&path)
        .execute_batch(&format!(
            "PRAGMA journal_mode = WAL; PRAGMA user_version = {PRE_BRIDGE}"
        ))
        .unwrap();
    let holder = open(&path);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap(); // a write here would wait, then fail
    let store = Store::open(&path, Duration::from_millis(200)).unwrap();
    assert!(store.list(false).unwrap().is_empty());
    drop(store);
    holder.execute_batch("ROLLBACK").unwrap();
    assert_eq!(stamp_of(&path), PRE_BRIDGE);
}

/// What a 0.10.5 process does on open (Python compat_prebridge.pre_bridge_open):
/// migrate under the write lock unless the stamp is its own, then stamp it.
fn pre_bridge_open(path: &Path, wait: Duration) -> rusqlite::Result<()> {
    let db = Connection::open(path)?;
    db.busy_timeout(wait)?;
    let stamp = |db: &Connection| db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0));
    if stamp(&db)? == PRE_BRIDGE {
        return Ok(());
    }
    db.execute_batch("BEGIN IMMEDIATE")?;
    if stamp(&db)? != PRE_BRIDGE {
        apply_schema(&db)?;
        db.execute_batch(&format!("PRAGMA user_version = {PRE_BRIDGE}"))?;
    }
    db.execute_batch("COMMIT")
}

/// Rounds per process, and the pause between: the three overlap without loading the machine.
const ROUNDS: usize = 40;
const PAUSE: Duration = Duration::from_millis(2);

#[test]
fn concurrent_pre_bridge_restamps_bridge_workers_and_hooks() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let wait = Duration::from_millis(500);
    let errors = Mutex::new(Vec::<String>::new());
    let written = Mutex::new(Vec::<i64>::new());
    let sql = |e: rusqlite::Error| {
        if !is_busy(&e) {
            errors.lock().unwrap().push(e.to_string());
        }
    };
    let hook = || match Store::open(&path, wait) {
        Ok(s) => {
            let insert =
                "INSERT INTO questions (created_at, question, norm) VALUES ('t', 'x', 'x')";
            match s.db().execute(insert, []) {
                Ok(_) => written.lock().unwrap().push(s.db().last_insert_rowid()),
                Err(e) => sql(e),
            }
        }
        Err(OpenError::Sqlite(e)) => sql(e),
        Err(e) => errors.lock().unwrap().push(e.to_string()), // the schema never goes away
    };
    let worker = || match Store::open_migrating(&path, wait) {
        Ok(_) => {}
        Err(MigrateError::Sqlite(e)) => sql(e),
        Err(e) => errors.lock().unwrap().push(e.to_string()),
    };
    let old = || {
        if let Err(e) = pre_bridge_open(&path, wait) {
            sql(e);
        }
    };
    let hard_stop = Instant::now() + Duration::from_secs(30);
    let wrote = || !written.lock().unwrap().is_empty();
    let looped = |f: &(dyn Fn() + Sync)| {
        let mut round = 0;
        while round < ROUNDS || (!wrote() && Instant::now() < hard_stop) {
            f();
            thread::sleep(PAUSE);
            round += 1;
        }
    };
    thread::scope(|s| {
        s.spawn(|| looped(&old));
        s.spawn(|| looped(&worker));
        s.spawn(|| looped(&hook));
    });
    assert_eq!(*errors.lock().unwrap(), Vec::<String>::new());
    let mut written = written.into_inner().unwrap();
    assert!(!written.is_empty());
    written.sort_unstable();
    let store = Store::open_migrating(&path, TIMEOUT).unwrap();
    let mut stmt = store
        .db()
        .prepare("SELECT id FROM questions ORDER BY id")
        .unwrap();
    let kept: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(kept, written);
    assert_eq!(stamp_of(&path), STAMP);
}

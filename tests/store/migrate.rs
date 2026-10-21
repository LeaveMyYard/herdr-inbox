//! The migrating open (worker and popup) against Python's: every captured
//! case (tests/fixtures/migrate.json, `tools/capture_goldens.py --only
//! schema`) is rebuilt from its SQL and stamp, checked against Python's
//! `describe_schema` before, migrated by `Store::open_migrating`, and
//! compared with what `Store(migrate=True)` left. The rest ports
//! tests/test_schema_migrations.py: forward only, re-checked under the lock,
//! no write lock when nothing is due, no data lost, concurrent opens.

use std::collections::BTreeMap;
use std::path::Path;
use std::thread;
use std::time::Duration;

use herdr_inbox::store::revision::STAMP;
use herdr_inbox::store::{OpenError, Store};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::common::{locked, migrated, open};

const CASES: &str = include_str!("../fixtures/migrate.json");
const TIMEOUT: Duration = Duration::from_secs(5);

fn doc() -> Value {
    serde_json::from_str(CASES).unwrap()
}

fn stamp_of(path: &Path) -> i64 {
    open(path)
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn set_stamp(path: &Path, value: i64) {
    open(path)
        .execute_batch(&format!("PRAGMA user_version = {value}"))
        .unwrap();
}

/// Python `store.describe_schema`, with the stamp, as the capture writes it.
fn describe(path: &Path) -> Value {
    let db = open(path);
    let mut objects = BTreeMap::new();
    let mut stmt = db
        .prepare("SELECT type, name, sql FROM sqlite_master")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (kind, name, sql) = row.unwrap();
        if name.starts_with("sqlite_") {
            continue;
        }
        let sql = sql
            .filter(|_| kind != "table")
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "));
        objects.insert(name, (kind, sql));
    }
    let mut columns = BTreeMap::new();
    for (name, (kind, _)) in &objects {
        if kind == "table" || kind == "view" {
            let mut stmt = db.prepare(&format!("PRAGMA table_info({name})")).unwrap();
            let cols: Vec<Value> = stmt
                .query_map([], |r| {
                    Ok(json!([
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)?
                    ]))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            columns.insert(name.clone(), Value::Array(cols));
        }
    }
    let user_version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    json!({"user_version": user_version, "columns": columns,
           "objects": objects.into_iter().map(|(k, (kind, sql))| (k, json!([kind, sql]))).collect::<BTreeMap<_, _>>()})
}

fn has_column(path: &Path, table: &str, column: &str) -> bool {
    let db = open(path);
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    names.iter().any(|n| n == column)
}

/// A database a release before `remote_outbox.sending_at` migrated and stamped
/// (Python compat_prebridge.make_older_head).
fn older_head(path: &Path, stamp: i64) {
    open(path)
        .execute_batch(&format!(
            "ALTER TABLE remote_outbox DROP COLUMN sending_at; PRAGMA user_version = {stamp}"
        ))
        .unwrap();
}

fn pre_bridge() -> i64 {
    doc()["current_pre_bridge"].as_i64().unwrap()
}

#[test]
fn every_captured_case_migrates_as_python_does() {
    let cases = doc()["cases"].as_array().unwrap().clone();
    assert!(cases.len() >= 7);
    for case in &cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inbox.db");
        let db = Connection::open(&path).unwrap();
        for sql in case["setup"].as_array().unwrap() {
            db.execute_batch(sql.as_str().unwrap()).unwrap();
        }
        db.execute_batch(&format!(
            "PRAGMA user_version = {}",
            case["before"]["user_version"]
        ))
        .unwrap();
        drop(db);
        let name = case["name"].as_str().unwrap();
        assert_eq!(
            describe(&path),
            case["before"],
            "{name}: rebuilt as captured"
        );
        drop(Store::open_migrating(&path, TIMEOUT).unwrap());
        assert_eq!(
            describe(&path),
            case["after"],
            "{name}: migrated as Python does"
        );
    }
}

#[test]
fn a_forward_migration_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    open(&path)
        .execute_batch(
            "INSERT INTO questions (created_at, question, norm) VALUES ('t', 'Kept?', 'kept')",
        )
        .unwrap();
    older_head(&path, pre_bridge());
    let store = Store::open_migrating(&path, TIMEOUT).unwrap();
    let kept: String = store
        .db()
        .query_row("SELECT question FROM questions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, "Kept?");
    assert!(has_column(&path, "remote_outbox", "sending_at"));
    assert_eq!(stamp_of(&path), STAMP);
}

#[test]
fn any_stamp_that_is_not_a_known_revision_migrates_forward() {
    let stamps = doc()["unknown_stamps"].as_array().unwrap().clone();
    assert!(stamps.len() >= 9);
    for stamp in stamps.iter().map(|s| s.as_i64().unwrap()) {
        let dir = tempfile::tempdir().unwrap();
        let path = migrated(dir.path());
        open(&path)
            .execute_batch(
                "INSERT INTO questions (created_at, question, norm) VALUES ('t', 'Kept?', 'kept')",
            )
            .unwrap();
        older_head(&path, stamp);
        let store = Store::open_migrating(&path, TIMEOUT).unwrap();
        let kept: String = store
            .db()
            .query_row("SELECT question FROM questions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, "Kept?", "{stamp}");
        assert!(has_column(&path, "remote_outbox", "sending_at"), "{stamp}");
        assert_eq!(stamp_of(&path), STAMP, "{stamp}");
    }
}

#[test]
fn the_migration_rechecks_under_the_lock_and_never_lowers_a_higher_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    set_stamp(&path, pre_bridge());
    let store = Store::open_migrating(&path, TIMEOUT).unwrap();
    assert_eq!(stamp_of(&path), STAMP);
    set_stamp(&path, STAMP + 1); // a newer worker stamped while this one waited
    older_head(&path, STAMP + 1);
    store.migrate().unwrap();
    assert_eq!(stamp_of(&path), STAMP + 1);
    assert!(!has_column(&path, "remote_outbox", "sending_at")); // nothing ran
}

#[test]
fn nothing_due_takes_no_write_lock() {
    // at this revision, or a higher one: opened while another process holds the write lock
    for stamp in [STAMP, STAMP + 1] {
        let dir = tempfile::tempdir().unwrap();
        let path = migrated(dir.path());
        set_stamp(&path, stamp);
        let holder = locked(&path, "IMMEDIATE");
        Store::open_migrating(&path, Duration::from_millis(200)).expect("no lock needed");
        holder.execute_batch("ROLLBACK").unwrap();
        assert_eq!(stamp_of(&path), stamp);
    }
}

#[test]
fn a_due_migration_behind_a_held_lock_fails_and_leaves_the_database_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    older_head(&path, pre_bridge());
    let holder = locked(&path, "IMMEDIATE");
    assert!(Store::open_migrating(&path, Duration::from_millis(200)).is_err());
    holder.execute_batch("ROLLBACK").unwrap();
    assert_eq!(stamp_of(&path), pre_bridge());
    assert!(!has_column(&path, "remote_outbox", "sending_at"));
}

#[test]
fn concurrent_migrating_opens_migrate_once_and_all_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    older_head(&path, 0);
    let opens: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            thread::spawn(move || Store::open_migrating(&path, TIMEOUT).map(drop))
        })
        .collect();
    for open in opens {
        open.join().unwrap().unwrap();
    }
    assert_eq!(stamp_of(&path), STAMP);
    assert!(has_column(&path, "remote_outbox", "sending_at"));
}

#[test]
fn the_migrating_open_creates_the_directory_and_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("herdr-inbox").join("inbox.db");
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    assert_eq!(stamp_of(&path), STAMP);
    // Python `_migrate`: `PRAGMA journal_mode=WAL` before the write lock
    let mode: String = open(&path)
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let fresh = &doc()["cases"][0];
    assert_eq!(fresh["name"], "fresh");
    assert_eq!(describe(&path), fresh["after"]);
}

#[test]
fn a_hook_open_never_migrates_and_works_once_the_worker_did() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    older_head(&path, pre_bridge());
    assert!(matches!(
        Store::open(&path, TIMEOUT),
        Err(OpenError::NotReady(_))
    ));
    assert_eq!(stamp_of(&path), pre_bridge());
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    Store::open(&path, TIMEOUT).expect("migrated");
}

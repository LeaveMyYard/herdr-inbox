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
use std::time::{Duration, Instant};

use herdr_inbox::store::migrate::switch_to_wal;
use herdr_inbox::store::owned::owned;
use herdr_inbox::store::revision::{BRIDGE_REVISION, REVISION_BASE, STAMP};
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

/// [`describe`], as Python's migration would have left it: what Rust added past the bridge
/// removed, and where Python stamps the bridge's revision, Rust's stamp checked and read as it.
fn as_bridge(path: &Path, python: &Value) -> Value {
    let mut got = describe(path);
    let own = owned();
    for table in own.tables.keys() {
        got["columns"].as_object_mut().unwrap().remove(table);
        got["objects"].as_object_mut().unwrap().remove(table);
    }
    for (table, added) in &own.added_columns {
        got["columns"][table]
            .as_array_mut()
            .unwrap()
            .retain(|c| !added.iter().any(|(name, _)| c[0] == name.as_str()));
    }
    let bridge = json!(REVISION_BASE + BRIDGE_REVISION);
    if python["user_version"] == bridge {
        assert_eq!(got["user_version"], STAMP);
        got["user_version"] = bridge;
    }
    got
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
            as_bridge(&path, &case["after"]),
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
    let started = Instant::now();
    assert!(Store::open_migrating(&path, Duration::from_millis(200)).is_err());
    let elapsed = started.elapsed();
    holder.execute_batch("ROLLBACK").unwrap();
    assert!(elapsed < Duration::from_millis(2500), "{elapsed:?}");
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
    assert_eq!(as_bridge(&path, &fresh["after"]), fresh["after"]);
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

#[test]
fn a_failed_migration_rolls_back_and_releases_the_write_lock() {
    // a caller that keeps its Store after a failed migrate() must not block the other workers
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let store = Store::open_migrating(&path, TIMEOUT).unwrap();
    open(&path)
        .execute_batch(
            "DROP TABLE remote_outbox; CREATE VIEW remote_outbox AS SELECT 1 AS request_id; PRAGMA user_version = 0",
        )
        .unwrap(); // a view where a table is: adding its columns fails mid-migration
    assert!(store.migrate().is_err());
    let other = open(&path);
    other.busy_timeout(Duration::ZERO).unwrap();
    other
        .execute_batch("BEGIN IMMEDIATE")
        .expect("the write lock is free");
    other.execute_batch("ROLLBACK").unwrap();
    assert_eq!(stamp_of(&path), 0); // nothing of the failed pass stayed
    drop(store);
}

/// The root cause of the concurrent-open flake: on a rollback-journal
/// database, the switch to WAL asks for the write lock while it holds a read
/// lock, and SQLite answers BUSY at once instead of waiting out the busy
/// timeout (it would deadlock with the writer).
#[test]
fn the_switch_to_wal_meets_a_writer_with_an_immediate_busy() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let holder = open(&path);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let db = open(&path);
    db.busy_timeout(TIMEOUT).unwrap();
    let started = std::time::Instant::now();
    let err = switch_to_wal(&db, Duration::ZERO).unwrap_err();
    assert!(herdr_inbox::store::busy::is_busy(&err), "{err}");
    assert!(started.elapsed() < TIMEOUT / 2, "no busy handler ran");
}

#[test]
fn a_migrating_open_waits_for_a_writer_on_a_rollback_journal_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    older_head(&path, 0);
    let mode: String = open(&path)
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "delete", "a fresh file, never switched to WAL");
    let holder = open(&path);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        holder.execute_batch("COMMIT").unwrap();
    });
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    release.join().unwrap();
    assert_eq!(stamp_of(&path), STAMP);
}

#[test]
fn the_switch_to_wal_returns_any_other_error_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    std::fs::write(&path, vec![b'x'; 4096]).unwrap(); // not a database
    let db = open(&path);
    let started = std::time::Instant::now();
    let err = switch_to_wal(&db, TIMEOUT).unwrap_err();
    assert!(!herdr_inbox::store::busy::is_busy(&err), "{err}");
    assert!(started.elapsed() < TIMEOUT / 2, "not retried");
}

/// The database a release at the bridge's revision left: every row of Python's schema,
/// nothing of Rust's, its stamp.
fn at_the_bridge(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("inbox.db");
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    let db = open(&path);
    db.execute_batch(
        "DROP TABLE question_host; DROP TABLE alert_host; \
         ALTER TABLE question_ext DROP COLUMN retry_count",
    )
    .unwrap();
    db.execute_batch(&format!(
        "PRAGMA user_version = {}",
        REVISION_BASE + BRIDGE_REVISION
    ))
    .unwrap();
    path
}

#[test]
fn a_bridge_database_gains_rusts_additions_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = at_the_bridge(dir.path());
    open(&path)
        .execute_batch(
            "INSERT INTO questions (created_at, question, norm) VALUES ('t', 'Kept?', 'kept'); \
             INSERT INTO question_ext (id, group_id, herdr_server) VALUES (1, 1, '/sock')",
        )
        .unwrap();
    let store = Store::open_migrating(&path, TIMEOUT).unwrap();
    assert_eq!(stamp_of(&path), STAMP);
    let (server, count): (String, Option<i64>) = store
        .db()
        .query_row(
            "SELECT herdr_server, retry_count FROM question_ext WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((server.as_str(), count), ("/sock", None));
    for table in ["question_host", "alert_host"] {
        let rows: i64 = store
            .db()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}

#[test]
fn rusts_additions_are_all_named_and_all_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    drop(Store::open_migrating(&path, TIMEOUT).unwrap());
    let own = owned();
    for (table, columns) in &own.tables {
        for column in columns {
            assert!(has_column(&path, table, column), "{table}.{column}");
        }
    }
    let described = describe(&path);
    for (table, columns) in &own.tables {
        assert_eq!(
            described["columns"][table].as_array().unwrap().len(),
            columns.len(),
            "{table} has only the columns named"
        );
    }
    assert!(has_column(&path, "question_ext", "retry_count"));
}

#[test]
fn raising_the_retry_count_is_a_change_a_reader_sees() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, TIMEOUT).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (created_at, question, norm) VALUES ('t', 'Q', 'q'); \
             INSERT INTO question_ext (id, group_id) VALUES (1, 1); DELETE FROM changes; \
             UPDATE question_ext SET retry_count = COALESCE(retry_count, 0) + 1 WHERE id = 1",
        )
        .unwrap();
    let seen: i64 = store
        .db()
        .query_row("SELECT COUNT(*) FROM changes WHERE qid = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(seen, 1, "the question_ext trigger covers the column");
}

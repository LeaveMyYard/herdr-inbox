//! Databases for the tests: built from the schema the Python migration
//! produces (captured in tests/fixtures/schema.json), never invented. Shared by
//! the `store` and `delivery` test binaries, each using part of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_inbox::store::readiness::expected;
use herdr_inbox::store::revision::{MIGRATION_REVISION, REVISION_BASE};
use rusqlite::{Connection, params};

pub const A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
pub const B: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// A database at `dir/inbox.db` migrated to this revision and stamped.
pub fn migrated(dir: &Path) -> PathBuf {
    let path = dir.join("inbox.db");
    let db = Connection::open(&path).unwrap();
    for sql in &expected().sql {
        db.execute_batch(sql).unwrap();
    }
    db.pragma_update(None, "user_version", REVISION_BASE + MIGRATION_REVISION)
        .unwrap();
    path
}

pub fn open(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

/// A question filed by `session`; returns its id.
pub fn ask(db: &Connection, session: &str, question: &str) -> i64 {
    db.execute(
        "INSERT INTO questions (created_at, session_id, question, norm) VALUES ('2026-10-17T10:00:00+00:00', ?, ?, ?)",
        params![session, question, question.to_lowercase()],
    )
    .unwrap();
    db.last_insert_rowid()
}

pub fn answer(db: &Connection, id: i64, text: &str) {
    db.execute(
        "UPDATE questions SET status = 'answered', answer = ?, answered_at = '2026-10-17T10:01:00+00:00' WHERE id = ?",
        params![text, id],
    )
    .unwrap();
}

/// A connection holding `path` in rollback-journal mode: `BEGIN EXCLUSIVE`
/// blocks readers and writers, `BEGIN IMMEDIATE` only writers (Python
/// test_hook._locked).
pub fn locked(path: &Path, mode: &str) -> Connection {
    let lock = Connection::open(path).unwrap();
    lock.pragma_update(None, "journal_mode", "DELETE").unwrap();
    lock.execute_batch(&format!("BEGIN {mode}")).unwrap();
    lock
}

/// What a locked database may cost a hook lookup: the 0.2 s deadline plus slack.
pub const LOOKUP_BUDGET: Duration = Duration::from_millis(500);

pub fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let out = f();
    (out, started.elapsed())
}

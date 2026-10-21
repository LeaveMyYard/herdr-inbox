//! Bringing the database forward (Python `Store(migrate=True)`,
//! `Store._migrate`, `_apply_schema`): only the delivery worker and the popup
//! open the database this way, and only they ever write the stamp.
//!
//! The migration is forward only and idempotent: under the write lock
//! (`BEGIN IMMEDIATE`) the stamp is read again, a database already at this
//! revision or a higher one is left exactly as it is, and anything else (a
//! legacy crc32 stamp, 0, a lower revision) gets every missing table,
//! column, index, trigger and view, then this revision's stamp. Several
//! processes opening an old database together serialize on the lock; every
//! one after the first finds the stamp and does nothing.
//!
//! The statements are Python's own, in Python's order, captured into
//! `tests/fixtures/migrate.json` (`tools/capture_goldens.py --only schema`):
//! the SQL a migration leaves in `sqlite_master` is then byte-identical.
//!
//! A hook that finds the database not migrated writes the revision it wants
//! (`revision_wanted`, #75); the worker reads that marker and hands over to
//! the newer worker (P5-E). Nothing here lowers or skips past a newer
//! release's stamp.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use super::connect::{Store, with_flags};
use super::revision::{MIGRATION_REVISION, STAMP, stamped_revision};

/// The captured migration of [`MIGRATION_REVISION`].
const MIGRATE_JSON: &str = include_str!("../../tests/fixtures/migrate.json");

/// Python's migration statements, in the order `_apply_schema` runs them.
#[derive(Debug, Deserialize)]
pub struct Migration {
    /// `_statements(SCHEMA)`: every `CREATE ... IF NOT EXISTS`.
    pub schema: Vec<String>,
    /// `TABLE_ADDED_COLUMNS`, in order: per table, the columns added after
    /// it first shipped, with their declarations.
    pub added_columns: Vec<(String, Vec<(String, String)>)>,
    /// `OLD_VIEWS`: views an earlier release created, dropped.
    pub old_views: Vec<String>,
    /// `_statements(VIEW_SQL)`: the read view.
    pub view: Vec<String>,
}

#[derive(Deserialize)]
struct Captured {
    revision: i64,
    migration: Migration,
}

/// This revision's migration (parsed once).
pub fn migration() -> &'static Migration {
    static MIGRATION: OnceLock<Migration> = OnceLock::new();
    MIGRATION.get_or_init(|| {
        let doc: Captured = serde_json::from_str(MIGRATE_JSON)
            .expect("tests/fixtures/migrate.json is the captured migration");
        assert_eq!(
            doc.revision, MIGRATION_REVISION,
            "migrate.json is another revision's"
        );
        doc.migration
    })
}

/// The column names of `table`.
fn columns(db: &Connection, table: &str) -> rusqlite::Result<BTreeSet<String>> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    stmt.query_map([], |r| r.get::<_, String>(1))?.collect()
}

/// Adds whichever of `added` the table lacks.
fn add_missing(db: &Connection, table: &str, added: &[(String, String)]) -> rusqlite::Result<()> {
    let have = columns(db, table)?;
    for (name, decl) in added {
        if !have.contains(name) {
            db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} {decl}"))?;
        }
    }
    Ok(())
}

/// Every statement that brings a database to this revision's schema (Python
/// `_apply_schema`), idempotent. The caller holds the transaction.
pub fn apply_schema(db: &Connection) -> rusqlite::Result<()> {
    let m = migration();
    for statement in &m.schema {
        db.execute_batch(statement)?;
    }
    for (table, added) in &m.added_columns {
        if table != "questions" {
            add_missing(db, table, added)?;
        }
    }
    for old in &m.old_views {
        db.execute_batch(&format!("DROP VIEW IF EXISTS {old}"))?;
    }
    for statement in &m.view {
        db.execute_batch(statement)?;
    }
    // `questions` after the view, as Python does: databases from before the column existed
    for (table, added) in &m.added_columns {
        if table == "questions" {
            add_missing(db, table, added)?;
        }
    }
    Ok(())
}

/// The revision the database is stamped with, if it is one.
fn revision(db: &Connection) -> rusqlite::Result<Option<i64>> {
    let user_version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(stamped_revision(user_version))
}

/// Not at this revision or a higher one: a migration is due.
fn due(db: &Connection) -> rusqlite::Result<bool> {
    Ok(revision(db)?.is_none_or(|r| r < MIGRATION_REVISION))
}

/// Why the migrating open failed.
#[derive(Debug)]
pub enum MigrateError {
    /// The database's directory could not be created.
    Io(std::io::Error),
    /// SQLite failed (opening, or the migration, which then rolled back).
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::Io(err) => err.fmt(f),
            MigrateError::Sqlite(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for MigrateError {}

impl From<rusqlite::Error> for MigrateError {
    fn from(err: rusqlite::Error) -> Self {
        MigrateError::Sqlite(err)
    }
}

impl Store {
    /// Opens `path` for the worker or the popup (Python `Store(path,
    /// timeout, migrate=True)` without its maintenance pass): creates the
    /// directory and the file if needed, and migrates a database not at this
    /// revision or a higher one.
    pub fn open_migrating(path: &Path, timeout: Duration) -> Result<Store, MigrateError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(MigrateError::Io)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
        let store = Store::from_connection(with_flags(path, flags, timeout)?);
        if due(store.db())? {
            store.migrate()?;
        }
        Ok(store)
    }

    /// Brings the schema forward to [`MIGRATION_REVISION`], once, under the
    /// write lock (Python `Store._migrate`): the stamp is read again there,
    /// and one at this revision or a higher one (a migration that ran while
    /// this waited, or a newer release's) is left alone.
    pub fn migrate(&self) -> rusqlite::Result<()> {
        let db = self.db();
        // cannot change inside a transaction
        db.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        db.execute_batch("BEGIN IMMEDIATE")?;
        let work = || -> rusqlite::Result<()> {
            if !due(db)? {
                return Ok(()); // someone migrated while we waited (or a newer release did)
            }
            apply_schema(db)?;
            db.execute_batch(&format!("PRAGMA user_version = {STAMP}"))
        };
        match work().and_then(|()| db.execute_batch("COMMIT")) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = db.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }
}

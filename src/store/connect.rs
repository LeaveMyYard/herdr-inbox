//! Opening the database: bare connections for the bounded lookups, and the
//! hook's [`Store`] (Python `Store(timeout=..., maintenance=False)`).

use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use super::readiness;

/// A read-only connection with SQLite's busy timeout at 0 (Python
/// `sqlite3.connect("file:...?mode=ro", uri=True, timeout=0)`), for
/// [`bounded`](super::busy::bounded) lookups.
pub fn read_only(path: &Path) -> rusqlite::Result<Connection> {
    with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY, Duration::ZERO)
}

/// A writable connection with SQLite's busy timeout at 0, for a bounded
/// write. Unlike Python's `sqlite3.connect`, it never creates the file.
pub fn read_write(path: &Path) -> rusqlite::Result<Connection> {
    with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE, Duration::ZERO)
}

/// Opens `path` as Python's `sqlite3` does: foreign keys NOT enforced (the
/// bundled SQLite is built with them on by default; Python's is not, and the
/// schema's `REFERENCES` were never enforced, e.g. a `question_ext` row may
/// precede or outlive its question).
fn with_flags(path: &Path, flags: OpenFlags, busy: Duration) -> rusqlite::Result<Connection> {
    let db = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(busy)?;
    db.pragma_update(None, "foreign_keys", false)?;
    Ok(db)
}

/// Why the hook's [`Store`] did not open.
#[derive(Debug)]
pub enum OpenError {
    /// The database lacks part of the schema this code uses (Python
    /// `SchemaNotReady`): not migrated yet, or migrated by an older release.
    /// The caller does nothing until the worker or the popup migrates it.
    NotReady(Vec<String>),
    /// SQLite failed (a missing file included: the hook never creates one).
    Sqlite(rusqlite::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::NotReady(missing) => {
                let shown: Vec<&str> = missing.iter().take(5).map(String::as_str).collect();
                write!(
                    f,
                    "inbox database not migrated yet: lacks {}",
                    shown.join(", ")
                )
            }
            OpenError::Sqlite(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<rusqlite::Error> for OpenError {
    fn from(err: rusqlite::Error) -> Self {
        OpenError::Sqlite(err)
    }
}

/// The database opened for a hook's writes: never migrates, never writes the
/// stamp, never runs maintenance. Opening checks the schema (see
/// [`readiness::missing`]); SQLite's own busy timeout governs every
/// statement, as in Python.
#[derive(Debug)]
pub struct Store {
    db: Connection,
}

impl Store {
    /// Opens `path` with SQLite's busy timeout at `timeout`.
    pub fn open(path: &Path, timeout: Duration) -> Result<Store, OpenError> {
        let db = with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE, timeout)?;
        let missing = readiness::missing(&db)?;
        if !missing.is_empty() {
            return Err(OpenError::NotReady(missing));
        }
        Ok(Store { db })
    }

    /// The connection, in autocommit mode.
    pub fn db(&self) -> &Connection {
        &self.db
    }

    /// `work` under the write lock (`BEGIN IMMEDIATE`), committed when it
    /// returns `Ok`, rolled back otherwise (Python `Store.transaction`).
    pub fn immediate<T>(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        match work(&self.db) {
            Ok(value) => {
                if let Err(err) = self.db.execute_batch("COMMIT") {
                    let _ = self.db.execute_batch("ROLLBACK");
                    return Err(err);
                }
                Ok(value)
            }
            Err(err) => {
                let _ = self.db.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }
}

//! The database's revision stamp, as the worker reads it (Python
//! `worker._db_stamp`).

use std::path::Path;
use std::time::Duration;

use crate::store::connect::read_only;
use crate::store::revision::stamped_revision;

/// How long the read waits for a writer (Python's `timeout=5`).
const BUSY: Duration = Duration::from_secs(5);

/// The revision `db` is stamped with: `None` for a legacy crc32 stamp, an
/// unstamped database, or one that cannot be read. Read-only: it never
/// creates or writes the file.
pub fn db_stamp(db: &Path) -> Option<i64> {
    let conn = read_only(db).ok()?;
    conn.busy_timeout(BUSY).ok()?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .ok()?;
    stamped_revision(version)
}

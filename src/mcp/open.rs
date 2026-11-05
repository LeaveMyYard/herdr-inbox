//! The store as a tool opens it (Python `server.tool_store`): never migrated,
//! never created, SQLite's busy timeout as given (a tool's:
//! [`TOOL_BUSY_TIMEOUT`](crate::mcp::retry::TOOL_BUSY_TIMEOUT)); and how
//! a store error reads to [`busy_safe`](super::retry::busy_safe).

use std::path::Path;

use std::time::Duration;

use crate::mcp::retry::Failure;
use crate::store::busy::is_busy;
use crate::store::{OpenError, Store};

/// A store error as one attempt's failure: busy or locked is retried,
/// anything else fails the call with SQLite's words.
pub fn failure(err: rusqlite::Error) -> Failure {
    if is_busy(&err) {
        Failure::Busy(err.to_string())
    } else {
        Failure::Error(err.to_string())
    }
}

/// Opens the database at `path` for one tool call, SQLite waiting up to
/// `busy_timeout` for a lock. A database not migrated
/// yet is [`Failure::NotReady`]; so is a missing file, which Python creates
/// empty and then finds not migrated (only the worker creates it here).
pub fn open(path: &Path, busy_timeout: Duration) -> Result<Store, Failure> {
    if !path.exists() {
        return Err(Failure::NotReady);
    }
    Store::open(path, busy_timeout).map_err(|e| match e {
        OpenError::NotReady(_) => Failure::NotReady,
        OpenError::Sqlite(e) => failure(e),
    })
}

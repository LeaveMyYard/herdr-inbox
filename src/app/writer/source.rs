//! What the writer reads: the inbox database, read-only and never migrated.
//! A database it cannot read makes a feed with `ok` false, never an empty one.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::ErrorCode;
use serde_json::Value;

use crate::app::feed::fit::MAX_FEED;
use crate::app::feed::snapshot;
use crate::store::{OpenError, Store};

/// How long a read waits for a lock before the feed says the database is busy.
const READ_WAIT: Duration = Duration::from_millis(250);

/// The database, kept open between polls.
#[derive(Debug)]
pub struct Source {
    path: PathBuf,
    store: Option<Store>,
    seen: Option<i64>,
}

impl Source {
    /// The database at `path`.
    pub fn new(path: &Path) -> Source {
        Source {
            path: path.to_path_buf(),
            store: None,
            seen: None,
        }
    }

    /// Whether another connection committed since the last call.
    pub fn changed(&mut self) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let now = store
            .db()
            .query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .ok();
        std::mem::replace(&mut self.seen, now) != now
    }

    /// The feed as it stands at `wall`, as the bytes to publish.
    pub fn feed(&mut self, wall: SystemTime) -> Vec<u8> {
        let feed = self.read(wall).unwrap_or_else(|why| {
            self.store = None;
            snapshot::unavailable(wall, why)
        });
        let bytes = serde_json::to_vec(&feed).unwrap_or_default();
        if bytes.len() > MAX_FEED || bytes.is_empty() {
            let refused = snapshot::unavailable(wall, "the feed did not fit in 1 MiB");
            return serde_json::to_vec(&refused).unwrap_or_default();
        }
        bytes
    }

    fn read(&mut self, wall: SystemTime) -> Result<Value, &'static str> {
        if self.store.is_none() {
            self.store = Some(open(&self.path)?);
        }
        let store = self.store.as_ref().ok_or("the inbox database is closed")?;
        snapshot::read(store, wall).map_err(|e| why(&e))
    }
}

fn open(path: &Path) -> Result<Store, &'static str> {
    if !path.exists() {
        return Err("the inbox database does not exist yet");
    }
    Store::open_read_only(path, READ_WAIT).map_err(|e| match e {
        OpenError::NotReady(_) => "the inbox database is not migrated to this version yet",
        OpenError::Sqlite(e) => why(&e),
    })
}

fn why(e: &rusqlite::Error) -> &'static str {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "the inbox database is busy",
        _ => "the inbox database could not be read",
    }
}

#[cfg(test)]
mod tests;

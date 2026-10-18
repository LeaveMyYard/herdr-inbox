//! The liveness stamps (Python `hook._touch`): the delivery hook fired, or
//! the rewake watcher is alive.
//!
//! The worker leaves a WORKING session's answers to its hooks only while
//! `hook_seen_at` is recent, and does not type into an idle session whose
//! watcher stamped `watch_seen_at` recently. Each is written at most once a
//! [`LIVENESS_EVERY`]: the common case is one read-only look.

use std::path::Path;
use std::time::{Duration, SystemTime};

use rusqlite::{OptionalExtension, params};

use super::busy::{PENDING_TIMEOUT, bounded};
use super::connect::{read_only, read_write};
use crate::clock::iso_seconds;

/// Seconds between a session's liveness writes (Python `LIVENESS_EVERY`).
pub const LIVENESS_EVERY: Duration = Duration::from_secs(60);

/// Which stamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// `sessions.hook_seen_at`: the PostToolBatch or Stop hook fired.
    HookSeen,
    /// `sessions.watch_seen_at`: the rewake watcher is alive (the IDLE
    /// gate's own column, never the tool hooks').
    WatchSeen,
}

impl Stamp {
    fn column(self) -> &'static str {
        match self {
            Stamp::HookSeen => "hook_seen_at",
            Stamp::WatchSeen => "watch_seen_at",
        }
    }
}

/// Stamps `stamp` for `session_id` at `now`, unless the stored one is under
/// [`LIVENESS_EVERY`] old. Silent: a missing, locked or older database means
/// the next firing tries again. Each lookup is bounded by
/// [`PENDING_TIMEOUT`] of wall clock.
pub fn touch(path: &Path, session_id: &str, stamp: Stamp, now: SystemTime) {
    if !path.exists() {
        return;
    }
    let column = stamp.column();
    let at = iso_seconds(now);
    let cutoff = iso_seconds(now - LIVENESS_EVERY);
    let read = format!("SELECT {column} FROM sessions WHERE session_id = ?");
    let stored = bounded(
        || read_only(path),
        |ro| {
            ro.query_row(&read, [session_id], |r| r.get::<_, Option<String>>(0))
                .optional()
        },
        PENDING_TIMEOUT,
    );
    let stored = match stored {
        Ok(row) => row.flatten(),
        Err(_) => return, // locked or not migrated yet
    };
    if stored.is_some_and(|s| !s.is_empty() && s >= cutoff) {
        return; // fresh enough: no write, no writable connection
    }
    let write = format!(
        "INSERT INTO sessions (session_id, {column}) VALUES (?, ?) ON CONFLICT(session_id) \
         DO UPDATE SET {column} = excluded.{column}"
    );
    // an upsert of the same stamp: safe to repeat
    let _ = bounded(
        || read_write(path),
        |rw| rw.execute(&write, params![session_id, at]).map(|_| ()),
        PENDING_TIMEOUT,
    );
}

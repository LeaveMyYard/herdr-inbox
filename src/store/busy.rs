//! Lookups bounded by wall clock (Python `hook._bounded`, herdr-inbox #45 and
//! #59).
//!
//! Not SQLite's busy timeout: it adds up the sleeps it asked for, not the
//! time they took (on macOS a 0.2 s timeout held the session rules about
//! 0.9 s). So a lookup opens its connection with the busy timeout at 0 and is
//! retried on a busy or locked database until the deadline has passed.

use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{Connection, Error};

/// The hook's read-only lookups give up after this (Python `PENDING_TIMEOUT`).
pub const PENDING_TIMEOUT: Duration = Duration::from_millis(200);

/// The sleep between attempts (Python `BUSY_STEP`).
pub const BUSY_STEP: Duration = Duration::from_millis(10);

/// SQLite's primary result codes for a busy and a locked database.
const SQLITE_BUSY: i32 = 5;
const SQLITE_LOCKED: i32 = 6;

/// Is `err` a busy or locked database? Judged by the primary code (the low
/// byte), so the extended codes (BUSY_RECOVERY, BUSY_SNAPSHOT,
/// LOCKED_SHAREDCACHE, ...) count too.
pub fn is_busy(err: &Error) -> bool {
    match err {
        Error::SqliteFailure(e, _) => {
            matches!(e.extended_code & 0xFF, SQLITE_BUSY | SQLITE_LOCKED)
        }
        _ => false,
    }
}

/// `work` on a fresh `connect()`, retried while the database is busy or
/// locked until `timeout` of wall clock has passed; then the last error.
/// Any other error is returned at once. `work` may run more than once, so
/// everything it does must be safe to repeat.
pub fn bounded<T>(
    mut connect: impl FnMut() -> rusqlite::Result<Connection>,
    mut work: impl FnMut(&Connection) -> rusqlite::Result<T>,
    timeout: Duration,
) -> rusqlite::Result<T> {
    let deadline = Instant::now() + timeout;
    loop {
        let err = match connect().and_then(|db| work(&db)) {
            Ok(value) => return Ok(value),
            Err(err) => err,
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if !is_busy(&err) || left.is_zero() {
            return Err(err);
        }
        thread::sleep(BUSY_STEP.min(left));
    }
}

#[cfg(test)]
mod tests;

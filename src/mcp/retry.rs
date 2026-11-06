//! Every tool call runs through [`busy_safe`] (Python `server.busy_safe`): a
//! busy or locked database is retried a few times and then answered with a
//! "busy" receipt, never a raw error; a database not migrated to this code's
//! schema yet gets the `unavailable` receipt ([`receipts::NOT_MIGRATED`])
//! after waiting a little for the worker. Tools never migrate: the worker
//! does, and each not-ready attempt asks for it (`ensure_worker`, #50).

use std::time::Duration;

use serde_json::Value;

use super::receipts;

/// Why one attempt of a tool did not return its result.
#[derive(Debug)]
pub enum Failure {
    /// The store's open found the schema incomplete (Python `SchemaNotReady`),
    /// before any write: safe to retry.
    NotReady,
    /// SQLite reported the database busy or locked, in these words.
    Busy(String),
    /// This process says it runs in a core terminal and that is refused: it
    /// has no identity, so every tool answers with this receipt.
    Refused(Value),
    /// Anything else: the call fails with this text (Python's exception).
    Error(String),
}

/// The waits between attempts.
#[derive(Clone, Debug)]
pub struct Backoff {
    /// After a busy attempt (Python `BUSY_BACKOFF`).
    pub busy: Vec<Duration>,
    /// After a not-ready attempt (Python `NOT_READY_BACKOFF`): together longer
    /// than a worker tick plus a handover and a migration.
    pub not_ready: Vec<Duration>,
    /// SQLite's busy timeout within one attempt ([`TOOL_BUSY_TIMEOUT`]).
    pub attempt: Duration,
}

impl Default for Backoff {
    fn default() -> Backoff {
        let s = Duration::from_secs_f64;
        Backoff {
            busy: vec![s(0.3), s(1.0), s(2.0)],
            not_ready: vec![s(0.3), s(1.0), s(2.0), s(3.0), s(3.0)],
            attempt: TOOL_BUSY_TIMEOUT,
        }
    }
}

/// SQLite's busy timeout for one attempt (Python `TOOL_BUSY_TIMEOUT`):
/// busy_safe retries, so a tool never hangs for minutes.
pub const TOOL_BUSY_TIMEOUT: Duration = Duration::from_secs(3);

/// `attempt` until it returns, as Python's `busy_safe` runs a tool: `Ok` is
/// the tool's result (a receipt included), `Err` a tool error's text.
///
/// A busy attempt waits its turn of `backoff.busy` and runs again; after the
/// last, the busy receipt. A not-ready attempt starts the not-ready waits:
/// each first asks for the worker, then waits and runs again; when they run
/// out, the not-migrated receipt. As in Python, a busy database during the
/// not-ready waits fails the call with SQLite's words (Python's
/// `OperationalError` escapes that inner loop).
pub fn busy_safe(
    mut attempt: impl FnMut() -> Result<Value, Failure>,
    backoff: &Backoff,
    ensure_worker: &dyn Fn(),
    sleep: &dyn Fn(Duration),
) -> Result<Value, String> {
    let mut not_ready = backoff.not_ready.iter();
    let mut busy = backoff.busy.iter();
    loop {
        match attempt() {
            Ok(v) => return Ok(v),
            Err(Failure::Refused(receipt)) => return Ok(receipt),
            Err(Failure::Error(e)) => return Err(e),
            Err(Failure::Busy(_)) => match busy.next() {
                Some(d) => sleep(*d),
                None => return Ok(receipts::busy()),
            },
            Err(Failure::NotReady) => {
                ensure_worker();
                for wait in not_ready.by_ref() {
                    sleep(*wait);
                    match attempt() {
                        Ok(v) => return Ok(v),
                        Err(Failure::Refused(receipt)) => return Ok(receipt),
                        Err(Failure::NotReady) => ensure_worker(),
                        Err(Failure::Busy(e)) => return Err(e),
                        Err(Failure::Error(e)) => return Err(e),
                    }
                }
                return Ok(receipts::not_migrated());
            }
        }
    }
}

#[cfg(test)]
mod tests;

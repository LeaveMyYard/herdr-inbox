//! What a worker does each tick (Python `worker.tick` and the `service`
//! passes it calls), for the herdr server it serves:
//!
//! - [`served`]: the pending answers this worker delivers, by pane (its own
//!   server's, plus pre-0.6.1 rows whose pane still holds the asking
//!   conversation); every other server's rows are left to that server's
//!   worker.
//! - [`retry`]: the retry pass over them, the agent-gone pass, and the one
//!   notification per pane for answers stuck too long ([`stuck`]).
//!
//! Everything goes through a [`Round`]: the delivery pass's [`Ctx`] and the
//! server's id, so the tests run a tick against a scripted herdr socket and
//! a temporary database.

pub mod retry;
pub mod served;
pub mod stuck;

use crate::delivery::run::Ctx;

/// What one tick works with.
pub struct Round<'r, 'a> {
    /// The delivery pass's world (store, herdr, transcripts, notifications).
    pub ctx: &'r mut Ctx<'a>,
    /// The herdr server this worker serves (Python `herdr.server_id()`).
    pub server: &'r str,
}

/// A tick step's failure, as Python's `log.warning(..., exc_info=True)`
/// writes it to the worker's log (its stderr).
pub fn warn(what: &str, err: &dyn std::fmt::Display) {
    eprintln!("herdr_inbox: {what}: {err}");
}

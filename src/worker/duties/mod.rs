//! What a worker does each tick (Python `worker.tick` and the `service`
//! passes it calls), for the herdr server it serves:
//!
//! - [`served`]: the pending answers this worker delivers, by pane (its own
//!   server's, plus pre-0.6.1 rows whose pane still holds the asking
//!   conversation); every other server's rows are left to that server's
//!   worker.
//! - [`retry`]: the retry pass over them, the agent-gone pass, and the one
//!   notification per pane for answers stuck too long ([`stuck`]).
//! - The reconcile family: [`unverified`] acknowledgements re-checked, the
//!   [`failed`] pile reconciled (forward family only), gone answers
//!   archived, conversations that [`moved`] revived and their continuations
//!   recorded, [`hooks`] deliveries verified.
//! - [`tick`]: all of it in Python's order.
//!
//! Everything goes through a [`Round`]: the delivery pass's [`Ctx`] and the
//! server's id, so the tests run a tick against a scripted herdr socket and
//! a temporary database.

pub mod alerts;
pub mod failed;
pub mod hooks;
pub mod moved;
pub mod retry;
pub mod served;
pub mod stuck;
pub mod tick;
pub mod unverified;

use std::collections::HashMap;
use std::time::Instant;

use crate::claude::evidence::Evidence;
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

/// What a worker remembers between ticks (Python's module-level
/// `_reconciled_at`, `_revive_checked`, `_starts_pruned_at` and
/// `transcript._evidence_cache`).
pub struct Memory {
    origin: Instant,
    /// When each failed answer was last looked for (monotonic seconds).
    pub reconciled_at: HashMap<i64, f64>,
    /// When each gone answer was last looked for in other panes.
    pub revive_checked: HashMap<i64, f64>,
    /// When the start markers were last pruned.
    pub starts_pruned_at: Option<f64>,
    /// The transcripts' evidence index.
    pub evidence: Evidence,
}

impl Default for Memory {
    fn default() -> Self {
        Memory {
            origin: Instant::now(),
            reconciled_at: HashMap::new(),
            revive_checked: HashMap::new(),
            starts_pruned_at: None,
            evidence: Evidence::default(),
        }
    }
}

impl Memory {
    /// Monotonic seconds since this memory began.
    pub fn monotonic(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }
}

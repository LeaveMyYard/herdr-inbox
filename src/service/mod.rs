//! Inbox operations: store changes plus their herdr side effects (Python
//! `service.py`), as the CLI uses them.
//!
//! - [`answer`]: answering (`submit`, `answer`) and closing (`close`).
//! - [`own`]: whether this process runs in the conversation that asked
//!   (`answered_by_own_conversation`).
//!
//! The herdr side goes through [`Delivery`] (the delivery pass and the
//! worker) and [`crate::surface::markers::Markers`] (the pane token and tab
//! prefix), so the tests drive these with the same stubs Python's goldens
//! use.

pub mod answer;
pub mod own;

use crate::store::{Question, Store};

/// The delivery side of an answer.
pub trait Delivery {
    /// Makes sure a delivery worker runs for `server` (`None`: this process's
    /// herdr server): it retries whatever a send leaves pending (Python
    /// `worker.ensure`, `service._ensure_worker`).
    fn ensure_worker(&mut self, server: Option<&str>);
    /// Delivers fresh answers now (Python `deliver_batch(store, qs)`: not a
    /// retry, not Send now, verified in this process). `Err`: the database
    /// failed; what was written before stays.
    fn deliver(&mut self, store: &Store, qs: &[Question]) -> rusqlite::Result<()> {
        self.deliver_as(store, qs, false, false)
    }
    /// [`Delivery::deliver`] as a retry (`retry`: the worker's own, which
    /// starts no worker) or the operator's Send now (`force`: no busy grace,
    /// no focused-pane wait) (Python `deliver_batch(store, qs, retry, force)`).
    fn deliver_as(
        &mut self,
        store: &Store,
        qs: &[Question],
        retry: bool,
        force: bool,
    ) -> rusqlite::Result<()>;
    /// Starts `inbox deliver <qids> [--force]` detached, so the caller returns
    /// at once and this machine's worker retries what it cannot deliver
    /// (Python `remote_api.spawn_delivery`, `service.spawn_cli`): `inbox
    /// apply` after a remote answer (`force`: the operator's Send now).
    fn spawn_deliver(&mut self, qids: &[i64], force: bool);
}

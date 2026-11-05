//! Handing recorded answers to delivery (Python `_send`'s detached branch).
//! **SAFETY**: the order is what keeps an answer from being stranded or
//! typed under the operator's fingers.
//!
//! 1. The delivery worker is made sure of first: if the popup dies before
//!    the next step, the worker retries what was recorded.
//! 2. Answers to the pane the popup returns to (the operator's) are typed
//!    now, while the popup still holds the keyboard; after it closes the
//!    operator may be typing there. Only their transcript check is handed
//!    off.
//! 3. The rest go to a detached `herdr-inbox deliver` (it outlives the
//!    popup; the delivery claims serialize it against every other sender).
//!
//! What carries each step out is the [`Handoff`] seam: the tests use a fake.

use crate::store::{Question, Store};

/// Delivery's side of a send.
pub trait Handoff: std::marker::Send + Sync {
    /// Makes sure a delivery worker runs (Python `service._ensure_worker`).
    fn ensure_worker(&self);
    /// Delivers `qs` (all to the operator's pane) in this process now, its
    /// check handed off (Python `deliver_batch(…, force=, verify_later=
    /// spawn_verify)`). `Err`: the database failed.
    fn deliver_here(&self, store: &Store, qs: &[Question], force: bool) -> rusqlite::Result<()>;
    /// Starts the detached sender for `qids` (Python `spawn_delivery`); a
    /// failure to start is only logged: a running worker retries them (the
    /// live `ensure_worker` starts none until P5-E2).
    fn spawn_delivery(&self, qids: &[i64], force: bool);
}

/// Hands `done` to delivery, in the order above. `focus_pane` is the pane
/// the popup returns to.
pub fn hand_off(
    store: &Store,
    done: &[Question],
    focus_pane: Option<&str>,
    force: bool,
    handoff: &dyn Handoff,
) -> rusqlite::Result<()> {
    if done.is_empty() {
        return Ok(());
    }
    handoff.ensure_worker();
    let ours = |q: &Question| {
        q.pane_id
            .as_deref()
            .is_some_and(|p| !p.is_empty() && Some(p) == focus_pane)
    };
    let here: Vec<Question> = done.iter().filter(|q| ours(q)).cloned().collect();
    if !here.is_empty() {
        handoff.deliver_here(store, &here, force)?;
    }
    let elsewhere: Vec<i64> = done.iter().filter(|q| !ours(q)).map(|q| q.id).collect();
    if !elsewhere.is_empty() {
        handoff.spawn_delivery(&elsewhere, force);
    }
    Ok(())
}

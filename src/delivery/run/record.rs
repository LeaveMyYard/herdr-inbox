//! The outcomes of a pane's pass, each carried out the moment the decision
//! settles it (Python writes it at that point of `deliver_batch`): a store
//! write under the pass's claim, or a verify-later job handed over right
//! after its prompt. Once the pass is halted, nothing more is written.

use inbox_core::deliver::{BUSY_WAIT_IDLE, Outcome};

use super::look::PaneLook;
use crate::store::{Claim, Store};

impl PaneLook<'_, '_> {
    /// Carries out answer `id`'s outcome; a failed write halts the pass.
    pub(super) fn record_outcome(&mut self, id: i64, outcome: &Outcome) {
        if self.halted() {
            return;
        }
        if *outcome == Outcome::VerifyLater {
            // building the job reads its plans: a failure there halts the
            // pass before the job goes out (Python raises before `verify_later`)
            if let Some(job) = self.job_of(id)
                && !self.halted()
                && let Some(later) = self.later.as_mut()
            {
                later(job);
            }
            return;
        }
        let Some(claim) = self.claims.get(&id) else {
            return;
        };
        if let Err(e) = write(self.ctx.store, id, claim, outcome) {
            self.error = Some(e);
        }
    }
}

/// The store write of one outcome, under the pass's claim.
fn write(store: &Store, id: i64, claim: &Claim, outcome: &Outcome) -> rusqlite::Result<()> {
    match outcome {
        Outcome::Untouched | Outcome::Moved | Outcome::VerifyLater => Ok(()),
        Outcome::Failed(error) => store.record_delivery(id, Some(error), true, Some(claim), false),
        Outcome::Delivered { ack } => store.record_delivery(id, None, *ack, Some(claim), true),
        Outcome::SentUnverified => store.record_delivery(id, None, false, Some(claim), false),
        Outcome::Pending(reason) => store.set_pending(id, claim, reason),
        Outcome::BusyMiss => store.record_busy_miss(id, claim, BUSY_WAIT_IDLE),
    }
}

//! The outcomes of a pane's pass, each carried out the moment the decision
//! settles it (Python writes it at that point of `deliver_batch`): a store
//! write under the pass's claim, or a verify-later job handed over right
//! after its prompt. Once the pass is halted, nothing more is written.

use std::time::Duration;

use inbox_core::deliver::{BUSY_WAIT_IDLE, NOT_IDLE, Outcome, STRANDED, TERMINAL_UNREAD};

use super::look::PaneLook;
use crate::clock::now_minus;
use crate::store::{Claim, Store};

/// The pending reason of the busy grace (`busy: the agent is <status>`).
const BUSY_AT: &str = "busy: the agent is ";

/// How long an answer may wait on a core terminal before it shows as not delivered.
pub const STRAND_AFTER: Duration = Duration::from_secs(300);

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
        let waits = self.core && self.waits_unseen(outcome);
        let stranded = waits.then(|| self.stranded(id, outcome)).flatten();
        let Some(claim) = self.claims.get(&id) else {
            return;
        };
        let outcome = stranded.as_ref().unwrap_or(outcome);
        // the outcome first: `set_pending` of a busy reason sets the clock as well
        let store = self.ctx.store;
        let written = write(store, id, claim, outcome).and_then(|()| match (self.core, waits) {
            (false, _) => Ok(()),
            (true, true) => store.start_waiting(id),
            (true, false) => store.stop_waiting(id),
        });
        if let Err(e) = written {
            self.error = Some(e);
        }
    }
}

impl PaneLook<'_, '_> {
    /// A wait with no end in sight: a core terminal that refuses (not `working`,
    /// which has its Stop hook to come) or cannot be read. A draft or a dialog has a
    /// person to clear them, and every other outcome ends or restarts the wait.
    fn waits_unseen(&self, outcome: &Outcome) -> bool {
        let Outcome::Pending(reason) = outcome else {
            return false;
        };
        let working = self.agent.as_ref().is_some_and(|a| a.status == "working");
        // `busy: the agent is error`: the grace before the send, on the same clock
        let refused = reason.starts_with(NOT_IDLE) || reason.starts_with(BUSY_AT);
        reason.starts_with(TERMINAL_UNREAD) || (refused && !working)
    }

    /// Such a wait that has gone on for [`STRAND_AFTER`], pass after pass, is no
    /// wait: the row then shows as not delivered and leaves the retry listing, so
    /// it costs no more `hc` runs. Its clock is `pending_since`, started by the
    /// first such pass and stopped by any other, so one bad read never strands it.
    fn stranded(&self, id: i64, outcome: &Outcome) -> Option<Outcome> {
        let Outcome::Pending(reason) = outcome else {
            return None;
        };
        let since = self.ctx.store.pending_since(id).ok().flatten()?;
        (!since.is_empty() && since <= now_minus(STRAND_AFTER))
            .then(|| Outcome::Failed(format!("{STRANDED}{}", plain(reason))))
    }
}

fn plain(reason: &str) -> &str {
    reason.strip_prefix("waiting: ").unwrap_or(reason)
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

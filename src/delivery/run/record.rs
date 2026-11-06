//! The outcomes of a pane's pass, each carried out the moment the decision
//! settles it (Python writes it at that point of `deliver_batch`): a store
//! write under the pass's claim, or a verify-later job handed over right
//! after its prompt. Once the pass is halted, nothing more is written.

use std::time::Duration;

use inbox_core::deliver::{BUSY_WAIT_IDLE, NOT_IDLE, Outcome, STRANDED, TERMINAL_UNREAD};

use super::look::PaneLook;
use crate::clock::now_minus;
use crate::store::{Claim, Store};

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
        let stranded = self.stranded(id, outcome);
        let Some(claim) = self.claims.get(&id) else {
            return;
        };
        let outcome = stranded.as_ref().unwrap_or(outcome);
        if let Err(e) = write(self.ctx.store, id, claim, outcome) {
            self.error = Some(e);
        }
    }
}

impl PaneLook<'_, '_> {
    /// A core terminal that has refused or been unreadable this long is no wait:
    /// nothing says it will end. The row then shows as not delivered, and leaves
    /// the retry listing, so it costs no more `hc` runs. A working terminal has
    /// its Stop hook to come, and a draft or a dialog a person to clear them.
    fn stranded(&self, id: i64, outcome: &Outcome) -> Option<Outcome> {
        let Outcome::Pending(reason) = outcome else {
            return None;
        };
        let working = self.agent.as_ref().is_some_and(|a| a.status == "working");
        let waiting = reason.starts_with(TERMINAL_UNREAD) || reason.starts_with(NOT_IDLE);
        let answered = self.q(id).answered_at.as_deref()?;
        (self.core && waiting && !working && answered <= now_minus(STRAND_AFTER).as_str())
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

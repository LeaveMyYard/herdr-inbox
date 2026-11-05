//! One tick, in Python `worker.tick`'s order: the unverified
//! acknowledgements re-checked, the failed pile reconciled (and gone
//! answers archived, old start markers pruned), moved conversations
//! revived, continuations adopted, hook deliveries verified, the blocked
//! scan, the change log trimmed, then the retry loop. A step Python runs in
//! its own `try` only logs its failure; the others fail the tick (the
//! worker then counts one answer pending and goes on).

use super::failed::{archive_gone, prune_starts, reconcile_failed};
use super::hooks::verify_hook_deliveries;
use super::moved::{adopt_continuations, revive_moved};
use super::retry::retry_round;
use super::unverified::reconcile_unverified;
use super::{Memory, Round, warn};

/// The change log keeps this many entries (Python's `DELETE FROM changes ...`).
pub const CHANGES_KEPT: i64 = 10_000;

/// What a tick found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tock {
    /// This worker's answers still pending.
    pub pending: usize,
    /// Agents herdr reported to the blocked scan (watched: they keep the worker alive).
    pub watching: usize,
}

fn step(what: &str, r: rusqlite::Result<()>) {
    if let Err(e) = r {
        warn(what, &e);
    }
}

/// One tick. `blocked_scan` is the blocked-agent scan (how many agents herdr
/// reports; its failure is its own to log, and counts none).
pub fn tick(
    round: &mut Round<'_, '_>,
    memory: &mut Memory,
    blocked_scan: &mut dyn FnMut(&mut Round<'_, '_>) -> usize,
) -> rusqlite::Result<Tock> {
    reconcile_unverified(round.ctx.store, round.ctx.files)?;
    step("failed-answer reconcile failed", {
        // Python: reconcile_failed, archive_gone and prune_starts in one `try`
        reconcile_failed(round, memory).and_then(|()| {
            archive_gone(round)?;
            prune_starts(round.ctx.state_dir, memory);
            Ok(())
        })
    });
    step("moved-pane scan failed", revive_moved(round, memory));
    step(
        "session continuation scan failed",
        adopt_continuations(round),
    );
    step(
        "hook delivery verification failed",
        verify_hook_deliveries(round),
    );
    let watching = blocked_scan(round);
    round.ctx.store.db().execute(
        "DELETE FROM changes WHERE seq <= (SELECT MAX(seq) FROM changes) - ?",
        [CHANGES_KEPT],
    )?;
    let pending = retry_round(round)?;
    Ok(Tock { pending, watching })
}

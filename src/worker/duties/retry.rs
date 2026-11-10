//! The retry loop over pending answers (the delivery half of Python
//! `worker.tick`): every pane this worker serves gets one retry pass, the
//! answers whose "no agent" / "conversation changed" retries ran out are
//! marked gone (`mark_gone`), and stuck answers raise their notification.

use inbox_core::deliver::{CHANGED, MAX_STALE_RETRIES, Mode, NO_AGENT};

use super::served::served;
use super::stuck::notify_stuck;
use super::{Round, warn};
use crate::delivery::run::deliver_batch;
use crate::store::Store;
use crate::store::hosts::CORE_SERVER;

/// One retry pass per served pane (Python `deliver_batch(store, items,
/// retry=True)`, verified in this process). One pane's failure never stops
/// the others: it is logged and the next pane goes on.
pub fn deliver_served(round: &mut Round<'_, '_>) -> rusqlite::Result<()> {
    let mode = Mode {
        retry: true,
        ..Mode::default()
    };
    for (pane, items) in served(round)? {
        if let Err(e) = deliver_batch(round.ctx, &items, mode, None) {
            warn(&format!("delivery to {pane} failed"), &e);
        }
    }
    Ok(())
}

/// Answers whose "no agent" / "conversation changed" retries ran out: the
/// agent is gone (Python `worker.mark_gone`). Marked once, then never polled
/// again; [`notify_stuck`] tells the operator once.
pub fn mark_gone(store: &Store) -> rusqlite::Result<()> {
    for q in store.gone_candidates(&[NO_AGENT, CHANGED], MAX_STALE_RETRIES)? {
        let reason = q
            .delivery_error
            .as_deref()
            .filter(|e| !e.is_empty())
            .unwrap_or("the agent is gone");
        store.mark_agent_gone(q.id, reason)?;
    }
    Ok(())
}

/// The stuck answers of this worker's server (or of none) get their
/// notification (Python `left = [...]; notify_stuck(store, left)`).
pub fn notify_left(round: &mut Round<'_, '_>) -> rusqlite::Result<()> {
    if round.server == CORE_SERVER {
        return Ok(()); // herdr's notice: the app has its own, and a legacy row is a herdr worker's
    }
    let store = round.ctx.store;
    let mut left = Vec::new();
    for q in store.stuck_answers()? {
        if store.server_of(q.id)?.is_none_or(|s| s == round.server) {
            left.push(q);
        }
    }
    notify_stuck(round, &left)
}

/// How many of this worker's answers are still pending (Python's tick
/// result; other servers' rows never keep it alive).
pub fn pending(round: &Round<'_, '_>) -> rusqlite::Result<usize> {
    Ok(served(round)?.iter().map(|(_, items)| items.len()).sum())
}

/// The retry half of a tick, in Python's order: deliver what this worker
/// serves, mark the gone, notify the stuck; the answers still pending.
pub fn retry_round(round: &mut Round<'_, '_>) -> rusqlite::Result<usize> {
    deliver_served(round)?;
    mark_gone(round.ctx.store)?;
    notify_left(round)?;
    pending(round)
}

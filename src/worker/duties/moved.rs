//! Conversations that moved (Python `service.revive_moved`,
//! `adopt_continuations`): an answer marked gone whose conversation now runs
//! in another pane of this server is moved there and delivered after all,
//! and every conversation with answers in play has its continuation
//! recorded (`sessions.continued_in`). Questions keep the session they were
//! filed under.

use std::time::Duration;

use super::{Memory, Round};
use crate::claude::chain;
use crate::delivery::run::{MOVED, session_pane, tab_name};
use crate::store::sessions::session_transcript;

/// A gone answer is looked for in other panes at most this often (Python `REVIVE_EVERY`).
pub const REVIVE_EVERY: f64 = 120.0;
/// Answers gone at most this long are looked for (Python `gone_recent(timedelta(days=1))`).
pub const REVIVE_WITHIN: Duration = Duration::from_secs(86_400);

/// One pass: moves the gone answers whose conversation runs elsewhere now.
pub fn revive_moved(round: &mut Round<'_, '_>, memory: &mut Memory) -> rusqlite::Result<()> {
    let store = round.ctx.store;
    let t = memory.monotonic();
    let mut rows = Vec::new();
    for q in store.gone_recent(REVIVE_WITHIN)? {
        let due = memory
            .revive_checked
            .get(&q.id)
            .is_none_or(|at| t - at >= REVIVE_EVERY);
        if store.server_of(q.id)?.is_none_or(|s| s == round.server) && due {
            rows.push(q);
        }
    }
    if rows.is_empty() {
        return Ok(());
    }
    for q in &rows {
        memory.revive_checked.insert(q.id, t);
    }
    let agents = crate::trace::herdr("agent.list", || round.ctx.herdr.agents()).unwrap_or_default();
    for q in rows {
        let Some((pane, tab)) = session_pane(round.ctx, q.session_id.as_deref(), &agents)? else {
            continue;
        };
        if Some(&pane) != q.pane_id.as_ref() {
            let label = tab_name(round.ctx, tab.as_deref());
            let reason = format!("{MOVED} {pane}");
            store.move_to_pane(q.id, &pane, tab.as_deref(), &reason, None, label.as_deref())?;
        }
    }
    Ok(())
}

/// Every tick, idempotent: each continuation of a session in play recorded.
pub fn adopt_continuations(round: &Round<'_, '_>) -> rusqlite::Result<()> {
    let (store, files) = (round.ctx.store, round.ctx.files);
    for session in store.sessions_in_play()? {
        let known = session_transcript(store.db(), &session)?;
        let found = chain(files, &session, known.as_deref());
        for pair in found.windows(2) {
            store.record_continuation(&pair[0], &pair[1])?;
        }
    }
    Ok(())
}

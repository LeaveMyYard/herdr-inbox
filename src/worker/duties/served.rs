//! The pending answers this worker delivers, by pane (Python
//! `worker.served`): its own herdr server's rows, plus rows asked before
//! 0.6.1 (no server recorded) whose pane here holds the asking conversation,
//! never on a pane-id-only match. Other servers' rows are theirs.

use std::time::Duration;

use inbox_core::deliver::{
    CHANGED, MAX_STALE_RETRIES, MAX_UNVERIFIED_ATTEMPTS, NO_AGENT, NOT_VERIFIED,
};

use super::Round;
use crate::delivery::pane::live_agent;
use crate::store::Question;
use crate::store::hosts::CORE_SERVER;

/// A pre-0.6.1 row's pane is looked at at most this often (Python
/// `LEGACY_RECHECK`).
pub const LEGACY_RECHECK: Duration = Duration::from_secs(60);

/// The errors given up after a number of tries (Python `worker.CAPS`).
pub const CAPS: [(&str, i64); 3] = [
    (NO_AGENT, MAX_STALE_RETRIES),
    (CHANGED, MAX_STALE_RETRIES),
    (NOT_VERIFIED, MAX_UNVERIFIED_ATTEMPTS),
];

/// The answers the core worker delivers: every core row of this machine, in one
/// list. The delivery pass groups them by terminal.
fn served_core(round: &Round<'_, '_>) -> rusqlite::Result<Vec<(String, Vec<Question>)>> {
    let rows = round.ctx.store.undelivered_core(&CAPS)?;
    Ok(if rows.is_empty() {
        Vec::new()
    } else {
        vec![(CORE_SERVER.to_owned(), rows)]
    })
}

/// The answers this worker delivers, per pane in the order the panes first
/// appear. A legacy row whose pane did not hold its conversation
/// [`MAX_STALE_RETRIES`] times is marked gone.
pub fn served(round: &Round<'_, '_>) -> rusqlite::Result<Vec<(String, Vec<Question>)>> {
    if round.server == CORE_SERVER {
        return served_core(round);
    }
    let store = round.ctx.store;
    let mut by_pane: Vec<(String, Vec<Question>)> = Vec::new();
    let mut legacy_ids: Vec<i64> = Vec::new();
    for q in store.undelivered_all(&CAPS)? {
        let server = store.server_of(q.id)?;
        if server.as_deref().is_some_and(|s| s != round.server) {
            continue; // another herdr server's pane: its own worker, never this socket
        }
        if server.is_none() {
            legacy_ids.push(q.id);
        }
        let pane = q.pane_id.clone().unwrap_or_default();
        match by_pane.iter_mut().find(|(p, _)| *p == pane) {
            Some((_, items)) => items.push(q),
            None => by_pane.push((pane, vec![q])),
        }
    }
    for (pane, items) in &mut by_pane {
        let legacy: Vec<&Question> = items
            .iter()
            .filter(|q| legacy_ids.contains(&q.id))
            .collect();
        if legacy.is_empty() {
            continue;
        }
        // pre-0.6.1 rows may belong to another herdr server: look at most every
        // LEGACY_RECHECK, and after MAX_STALE_RETRIES misses call the agent gone
        let mut due = Vec::new();
        for q in legacy {
            if store.legacy_check_due(q.id, LEGACY_RECHECK)? {
                due.push(q);
            }
        }
        let mut ok: Vec<i64> = Vec::new();
        if !due.is_empty() {
            let ctx = &round.ctx;
            let agent = live_agent(ctx.herdr, ctx.files, ctx.state_dir, pane);
            for q in due {
                let asker = q.session_id.as_deref().filter(|s| !s.is_empty());
                let live = agent.as_ref().and_then(|a| a.session.as_deref());
                if asker.is_some() && asker == live {
                    ok.push(q.id);
                } else if store.legacy_miss(q.id)? >= MAX_STALE_RETRIES {
                    let reason = format!("no agent of that conversation in pane {pane}");
                    store.mark_agent_gone(q.id, &reason)?;
                }
            }
        }
        items.retain(|q| !legacy_ids.contains(&q.id) || ok.contains(&q.id));
    }
    by_pane.retain(|(_, items)| !items.is_empty());
    Ok(by_pane)
}

//! The blocked-agent scan's herdr side (Python `alerts.scan_blocked`'s
//! calls): one `agent list`, the scan of
//! [`crate::store::alerts::BlockedScan`], and its effects (a
//! notification, the pane's marks). Nothing is ever typed into a pane at a
//! prompt. How many agents herdr reports keeps the worker alive: their
//! blocked state is watched.

use herdr_agent_kit::herdr_api::TabId;
use serde_json::{Value, json};

use super::{Round, warn};
use crate::store::alerts::{BlockedScan, Effect, SeenAgent};

/// One scan: the agents herdr reports (0 when it does not answer, or when
/// the scan fails, which is logged). `marks` refreshes a pane's marks.
pub fn scan_blocked(
    round: &mut Round<'_, '_>,
    scan: &mut BlockedScan,
    t: f64,
    marks: &mut dyn FnMut(Option<&str>),
) -> usize {
    let herdr = round.ctx.herdr;
    // Python `herdr.call("agent", "list")`: None on a failure, else `.result`
    // a missing `agents` is none; one that is not a list is no answer (Python
    // raises iterating it, and the scan changes nothing)
    let agents: Option<Vec<SeenAgent>> =
        herdr
            .raw("agent.list", json!({}))
            .ok()
            .and_then(|res| match res.get("agents") {
                None => Some(Vec::new()),
                Some(Value::Array(a)) => Some(a.iter().filter_map(SeenAgent::from_json).collect()),
                Some(_) => None,
            });
    let label = |tab: &str| {
        herdr
            .tab(&TabId::new(tab))
            .ok()
            .and_then(|t| t.label)
            .filter(|l| !l.is_empty())
    };
    match scan.scan(round.ctx.store, round.server, agents.as_deref(), t, label) {
        Ok((n, effects)) => {
            for e in effects {
                match e {
                    Effect::Notify { title, body } => (round.ctx.notify)(&title, &body),
                    Effect::Markers(pane) => marks(pane.as_deref()),
                }
            }
            n
        }
        Err(e) => {
            warn("blocked-agent scan failed", &e);
            0
        }
    }
}

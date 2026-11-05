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

/// `agent.list`'s result (Python `(result or {}).get("agents", [])`): a falsy
/// result or a missing `agents` is none; anything else that is not a list is
/// no answer (Python raises on it, and the scan changes nothing).
fn seen(result: &Value) -> Option<Vec<SeenAgent>> {
    match result {
        Value::Object(o) => match o.get("agents") {
            None => Some(Vec::new()),
            Some(Value::Array(a)) => Some(a.iter().filter_map(SeenAgent::from_json).collect()),
            Some(_) => None,
        },
        Value::Null | Value::Bool(false) => Some(Vec::new()),
        Value::Array(a) if a.is_empty() => Some(Vec::new()),
        Value::String(s) if s.is_empty() => Some(Vec::new()),
        Value::Number(n) if n.as_f64() == Some(0.0) => Some(Vec::new()),
        _ => None,
    }
}

/// One scan: the agents herdr reports (0 when it does not answer, or when
/// the scan fails, which is logged after the effects of what it had already
/// committed are carried out). `marks` refreshes a pane's marks.
pub fn scan_blocked(
    round: &mut Round<'_, '_>,
    scan: &mut BlockedScan,
    t: f64,
    marks: &mut dyn FnMut(Option<&str>),
) -> usize {
    let herdr = round.ctx.herdr;
    // Python `herdr.call("agent", "list")`: None on a failure, else `.result`
    let agents = herdr
        .raw("agent.list", json!({}))
        .ok()
        .and_then(|r| seen(&r));
    let label = |tab: &str| {
        herdr
            .tab(&TabId::new(tab))
            .ok()
            .and_then(|t| t.label)
            .filter(|l| !l.is_empty())
    };
    let scanned = scan.scan(round.ctx.store, round.server, agents.as_deref(), t, label);
    let (n, effects) = scanned.value;
    for e in effects {
        match e {
            Effect::Notify { title, body } => (round.ctx.notify)(&title, &body),
            Effect::Markers(pane) => marks(pane.as_deref()),
        }
    }
    if let Some(e) = scanned.error {
        warn("blocked-agent scan failed", &e);
    }
    n
}

//! The decision, step for step with Python `service.deliver_batch` for one
//! pane: each answer leaves at the first step that settles it, and a lookup
//! is made only when a step needs it.

use std::collections::BTreeMap;

use super::reasons::{
    BLOCKED_WAIT, BUSY_WAIT_IDLE, CHANGED, HOOK_WAIT, NO_AGENT, NO_PANE, OPERATOR_IN_PANE, PANEL,
    REQUEUED_WAIT, WATCHER_WAIT, older_error,
};
use super::typing::type_them;
use super::{Agent, Look, MAX_BUSY_ATTEMPTS, MAX_HOOK_MISSES, Mode, Outcome, Waiting};
use crate::REFERENCE_INDEX;

/// The statuses at which a prompt is typed without waiting (Python `DELIVERABLE`).
pub(super) const DELIVERABLE: [&str; 2] = ["idle", "done"];

pub(super) type Outcomes = BTreeMap<i64, Outcome>;

/// What becomes of each of `pane`'s waiting answers (`None`: no pane known),
/// in their order.
pub fn decide(
    pane: Option<&str>,
    waiting: &[Waiting],
    mode: Mode,
    look: &mut impl Look,
) -> Vec<(i64, Outcome)> {
    let mut out = Outcomes::new();
    // every row is claimed before any outcome is written for it
    let claimed: Vec<&Waiting> = waiting.iter().filter(|q| look.claimed(q.id)).collect();
    if !claimed.is_empty() {
        settle(pane, claimed, mode, look, &mut out);
    }
    waiting
        .iter()
        .map(|q| (q.id, out.remove(&q.id).unwrap_or(Outcome::Untouched)))
        .collect()
}

fn settle(
    pane: Option<&str>,
    items: Vec<&Waiting>,
    mode: Mode,
    look: &mut impl Look,
    out: &mut Outcomes,
) {
    let Some(pane) = pane.filter(|p| !p.is_empty()) else {
        for q in items {
            out.insert(q.id, Outcome::Failed(NO_PANE.into()));
        }
        return;
    };
    let Some(agent) = look.agent() else {
        for q in items {
            // its pane may only have moved
            let o = if look.moved_elsewhere(q.id) {
                Outcome::Moved
            } else {
                Outcome::Failed(format!("{NO_AGENT} {pane}"))
            };
            out.insert(q.id, o);
        }
        return;
    };
    let ours = ours_only(pane, &agent, items, look, out);
    if ours.is_empty() {
        return;
    }
    let ours = before_typing(&agent, ours, mode, look, out);
    if ours.is_empty() {
        return;
    }
    let ours = not_yet_there(pane, &agent, ours, look, out);
    if !ours.is_empty() {
        type_them(&agent, &ours, mode, look, out);
    }
}

/// Answers whose conversation the pane runs (the asker's or one it continued
/// into); the others moved or are recorded as changed or older.
fn ours_only<'a>(
    pane: &str,
    agent: &Agent,
    items: Vec<&'a Waiting>,
    look: &mut impl Look,
    out: &mut Outcomes,
) -> Vec<&'a Waiting> {
    let mut ours = Vec::new();
    let mut changed = Vec::new();
    for q in items {
        if look.conversation_changed(q.id) {
            changed.push(q);
        } else {
            ours.push(q);
        }
    }
    for q in changed {
        if look.moved_elsewhere(q.id) {
            out.insert(q.id, Outcome::Moved);
        } else if look.pane_runs_older_session(q.id) {
            out.insert(
                q.id,
                Outcome::Failed(older_error(
                    pane,
                    agent.session.as_deref(),
                    q.session.as_deref(),
                    false,
                )),
            );
        } else {
            out.insert(q.id, Outcome::Failed(format!("{CHANGED} {pane}")));
        }
    }
    ours
}

/// The waits before anything is typed: a prompt or dialog, the session's own
/// watcher or hook, a busy agent, the operator in the pane, the panel.
fn before_typing<'a>(
    agent: &Agent,
    mut ours: Vec<&'a Waiting>,
    mode: Mode,
    look: &mut impl Look,
    out: &mut Outcomes,
) -> Vec<&'a Waiting> {
    let status = agent.status.as_str();
    let pending = |ours: &[&Waiting], reason: &str, out: &mut Outcomes| {
        for q in ours {
            out.insert(q.id, Outcome::Pending(reason.to_string()));
        }
    };
    if status == "blocked" {
        pending(&ours, BLOCKED_WAIT, out);
        return Vec::new();
    }
    let deliverable = DELIVERABLE.contains(&status);
    if deliverable && !mode.force && look.watched() {
        // left to its rewake watcher, unless its hooks missed it twice
        let mut rest = Vec::new();
        for q in ours {
            if look.hook_misses(q.id) < MAX_HOOK_MISSES {
                out.insert(q.id, Outcome::Pending(WATCHER_WAIT.into()));
            } else {
                rest.push(q);
            }
        }
        ours = rest;
        if ours.is_empty() {
            return ours;
        }
    }
    if status == "working" && !mode.force && look.hook_live() {
        // its own hook carries the answer, unless it missed it twice
        let mut rest = Vec::new();
        for q in ours {
            if look.hook_misses(q.id) < MAX_HOOK_MISSES {
                out.insert(q.id, Outcome::Pending(HOOK_WAIT.into()));
            } else {
                rest.push(q);
            }
        }
        ours = rest;
        if ours.is_empty() {
            return ours;
        }
    }
    let busy = !deliverable;
    if busy && !mode.force {
        // requeued after an old server's lost delivery: at rest only
        let (requeued, rest): (Vec<&Waiting>, Vec<&Waiting>) =
            ours.into_iter().partition(|q| q.requeued);
        pending(&requeued, REQUEUED_WAIT, out);
        ours = rest;
        if ours.is_empty() {
            return ours;
        }
    }
    if busy {
        let mut rest = Vec::new();
        for q in ours {
            if !mode.force && look.busy_attempts(q.id) >= MAX_BUSY_ATTEMPTS {
                out.insert(q.id, Outcome::Pending(BUSY_WAIT_IDLE.into()));
            } else if !mode.force && look.within_busy_grace(q.id) {
                out.insert(
                    q.id,
                    Outcome::Pending(format!("busy: the agent is {status}")),
                );
            } else {
                rest.push(q);
            }
        }
        ours = rest;
        if ours.is_empty() {
            return ours;
        }
    }
    if mode.retry && agent.focused && !mode.force {
        pending(&ours, OPERATOR_IN_PANE, out);
        return Vec::new();
    }
    if !mode.force && look.panel_focused() {
        pending(&ours, PANEL, out);
        return Vec::new();
    }
    ours
}

/// Answers not already in the conversation (a late landing), nor already
/// typed into an older session the pane reports.
fn not_yet_there<'a>(
    pane: &str,
    agent: &Agent,
    ours: Vec<&'a Waiting>,
    look: &mut impl Look,
    out: &mut Outcomes,
) -> Vec<&'a Waiting> {
    let ids: Vec<i64> = ours.iter().map(|q| q.id).collect();
    let landed = look.landed(&ids);
    let mut rest = Vec::new();
    for (q, found) in ours.into_iter().zip(landed) {
        match found {
            Some(segments) => {
                // only the long-answer reference landed: verified, but it stays unseen
                let ack = segments.iter().any(|s| *s != REFERENCE_INDEX);
                out.insert(q.id, Outcome::Delivered { ack });
            }
            None => rest.push(q),
        }
    }
    if rest.is_empty() || !look.pane_reports_another_session() {
        return rest;
    }
    let mut left = Vec::new();
    for q in rest {
        if look.already_typed_into_older_session(q.id) {
            let error = older_error(
                pane,
                agent.reported_session.as_deref(),
                q.session.as_deref(),
                true,
            );
            out.insert(q.id, Outcome::Failed(error));
        } else {
            left.push(q);
        }
    }
    left
}

//! One notification per pane (and kind) for answers stuck too long or out
//! of tries (Python `worker.notify_stuck`, `_ids`, `_where`). An agent
//! waiting at a prompt is pointed at; the rest can use `check_answers`.

use std::time::Duration;

use inbox_core::deliver::{MAX_UNVERIFIED_ATTEMPTS, NOT_VERIFIED, OLDER};

use super::Round;
use crate::clock::now_minus;
use crate::delivery::run::tab_name;
use crate::store::Question;
use crate::store::outcomes::AGENT_GONE;

/// An answer pending this long raises its notification (Python `NOTIFY_AFTER`).
pub const NOTIFY_AFTER: Duration = Duration::from_secs(300);

/// What a stuck answer is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Its agent is gone.
    Gone,
    /// Its agent is at a prompt.
    Blocked,
    /// Its pane runs an older session of the conversation.
    Older,
    /// Anything else.
    Stuck,
}

fn error(q: &Question) -> &str {
    q.delivery_error.as_deref().unwrap_or("")
}

/// The kind of a stuck answer (Python's `_gone`, `_blocked`, `OLDER` checks).
pub fn kind(q: &Question) -> Kind {
    let e = error(q);
    if e.starts_with(AGENT_GONE) {
        Kind::Gone
    } else if e.starts_with("pending: blocked:") && !e.contains("panel") {
        Kind::Blocked
    } else if e.starts_with(OLDER) {
        Kind::Older
    } else {
        Kind::Stuck
    }
}

/// `Q3–Q5` for a run of consecutive ids, else `Q3, Q7` (Python `_ids`).
pub fn ids(qids: &[i64]) -> String {
    let mut q = qids.to_vec();
    q.sort_unstable();
    let run = q.len() > 1 && q.windows(2).all(|w| w[1] == w[0] + 1);
    if run {
        return format!("Q{}–Q{}", q[0], q[q.len() - 1]);
    }
    q.iter()
        .map(|i| format!("Q{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Where the answer waits: its tab's label now (looked up unless `lookup`
/// is false), else the label it was asked under, its pane, "its pane"
/// (Python `_where`).
fn place(round: &Round<'_, '_>, q: &Question, lookup: bool) -> String {
    let now = if lookup {
        tab_name(round.ctx, q.tab_id.as_deref())
    } else {
        None
    };
    [now, q.tab_label.clone(), q.pane_id.clone()]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "its pane".into())
}

/// The notification for `kind`'s answers `ids` in `place`: title and body.
pub fn message(kind: Kind, ids: &str, place: &str) -> (String, String) {
    let (title, body) = match kind {
        Kind::Gone => (
            format!("{ids}: the agent in {place} is gone"),
            "The answer is kept; a new session can fetch it with check_answers.",
        ),
        Kind::Older => (
            format!("{ids}: {place} runs an older conversation"),
            "herdr resumed an earlier session of the asking agent; resume the asking session to \
             deliver (the inbox shows its id), or fetch with check_answers.",
        ),
        Kind::Blocked => (
            format!("waiting: {place} is at a prompt ({ids})"),
            "Answer the prompt in that pane; the inbox delivers right after.",
        ),
        Kind::Stuck => (
            format!("{ids} not delivered to {place}"),
            "The agent can fetch them with check_answers; or retry from the inbox.",
        ),
    };
    (title, body.to_string())
}

/// Notifies once per pane and kind about `rows` stuck [`NOTIFY_AFTER`] or
/// out of tries, gone answers included; each is notified once.
pub fn notify_stuck(round: &mut Round<'_, '_>, rows: &[Question]) -> rusqlite::Result<()> {
    let store = round.ctx.store;
    let old = now_minus(NOTIFY_AFTER);
    let mut groups: Vec<((String, Kind), Vec<&Question>)> = Vec::new();
    for q in rows {
        let out_of_tries =
            error(q) == NOT_VERIFIED && q.delivery_attempts >= MAX_UNVERIFIED_ATTEMPTS;
        let k = kind(q);
        // Python `_age(answered_at) >= NOTIFY_AFTER` (no time: age 0)
        let long = q
            .answered_at
            .as_deref()
            .is_some_and(|a| !a.is_empty() && *a <= *old);
        if (out_of_tries || k == Kind::Gone || long) && !store.notified(q.id)? {
            let key = (q.pane_id.clone().unwrap_or_else(|| "-".into()), k);
            match groups.iter_mut().find(|(g, _)| *g == key) {
                Some((_, items)) => items.push(q),
                None => groups.push((key, vec![q])),
            }
        }
    }
    for ((_, k), items) in groups {
        let qids: Vec<i64> = items.iter().map(|q| q.id).collect();
        let place = place(round, items[0], k != Kind::Gone);
        let (title, body) = message(k, &ids(&qids), &place);
        (round.ctx.notify)(&title, &body);
        store.mark_notified(&qids)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

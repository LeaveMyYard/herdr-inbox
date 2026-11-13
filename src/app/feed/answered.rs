//! The feed's `answered` rows (`app-feed-v1.md`, "Answered"): what the
//! operator answered lately, for a chat view's "✓ Title · Option B" line.
//!
//! **SAFETY**: a row says that a question was answered and which of its own
//! option labels were picked, and nothing the operator typed: no free text,
//! no notes, not the stored answer. Whether there were any is one boolean.
//! It takes no verb, so it has no token.

use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::asker;
use crate::clock::iso_seconds;
use crate::store::Store;
use crate::store::question::Question;
use crate::tui::snapshot::convert::detail;

/// The most rows per asker.
pub const PER_ASKER: usize = 20;
/// How far back an answer is shown.
pub const WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// The rows at wall-clock time `wall`: answered within [`WINDOW`], newest
/// answer first, at most [`PER_ASKER`] per asker.
pub fn rows(store: &Store, wall: SystemTime) -> rusqlite::Result<Vec<Value>> {
    let since = iso_seconds(wall.checked_sub(WINDOW).unwrap_or(SystemTime::UNIX_EPOCH));
    let mut seen: Vec<(Value, usize)> = Vec::new();
    let mut out = Vec::new();
    for q in store.answered_since(&since)? {
        // the asker first, so a row past its asker's cap costs one lookup and is never built
        let asker = asker::of_question(store, &q)?;
        let who = who(&asker);
        let count = match seen.iter_mut().find(|(k, _)| *k == who) {
            Some((_, n)) => n,
            None => {
                seen.push((who, 0));
                &mut seen.last_mut().expect("just pushed").1
            }
        };
        if *count < PER_ASKER {
            *count += 1;
            out.push(row(store, &q, asker)?);
        }
    }
    Ok(out)
}

/// An asker as one agent: its host and where it runs, not its tab's label
/// or folder at the time.
fn who(asker: &Value) -> Value {
    json!([asker["host"], asker["pane"], asker["terminal"]])
}

/// The row of answered question `q`, asked by `asker` ([`asker::of_question`]).
pub fn row(store: &Store, q: &Question, asker: Value) -> rusqlite::Result<Value> {
    let (choices, typed) = picked(q);
    Ok(json!({
        "key": format!("answered:{}", q.id),
        "kind": "answered",
        "question_id": q.id,
        "group": q.group(),
        "title": q.group_title,
        "header": q.header,
        "asked_by": q.asked_by,
        "created_at": q.created_at,
        "answered_at": q.answered_at,
        "choices": choices,
        "typed": typed,
        "delivered": q.delivered_at.is_some() || q.acknowledged_at.is_some(),
        "tool_use_id": store.tool_use_id(q.id)?,
        "asker": asker,
    }))
}

/// The option labels picked, in option order, and whether the operator also
/// typed something (text or a note). A label the question does not have is
/// never published: only the question's own labels are. An old answer with
/// no parts counts as one picked label when it is exactly one, and as typed
/// otherwise.
fn picked(q: &Question) -> (Vec<String>, bool) {
    let labels: Vec<&str> = q
        .options
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    let blank = |s: &str| s.trim().is_empty();
    match q.answer_json.as_ref().and_then(detail) {
        Some(d) => {
            let choices = labels
                .iter()
                .filter(|l| d.choices.iter().any(|c| c == *l))
                .map(|l| (*l).to_string())
                .collect();
            let typed = d.text.as_deref().is_some_and(|t| !blank(t))
                || d.notes.iter().any(|(_, n)| !blank(n));
            (choices, typed)
        }
        None => match q.answer.as_deref() {
            Some(a) if labels.contains(&a) => (vec![a.to_string()], false),
            Some(a) => (Vec::new(), !blank(a)),
            None => (Vec::new(), false),
        },
    }
}

#[cfg(test)]
mod tests;

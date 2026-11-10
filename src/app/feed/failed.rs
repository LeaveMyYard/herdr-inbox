//! A `failed` row: an answer the agent has not had, and where its delivery
//! stands. The states and `wait` words are the feed's own reading of the
//! stored `delivery_error`, so a mod never parses a reason.

use inbox_core::deliver::{BUSY_WAIT_IDLE, OPERATOR_IN_PANE, PANEL};
use serde_json::{Value, json};

use super::asker;
use super::text::{LONG, Text};
use super::token::token;
use crate::store::Store;
use crate::store::outcomes::AGENT_GONE;
use crate::store::question::Question;
use crate::store::questions::PENDING;

/// What the row takes.
pub const ACTIONS: [&str; 2] = ["retry", "stop-showing"];

/// The row of answered question `q`. `auto_retry`: it is in a worker's retry
/// listing. The token does not cover it, so a verb can pass anything.
pub fn entry(store: &Store, q: &Question, auto_retry: bool) -> rusqlite::Result<Value> {
    let mut cut = false;
    let question = Text::Cut.apply(&q.question, LONG, &mut cut);
    let answer = Text::Cut.apply_opt(q.answer.as_deref(), LONG, &mut cut);
    let mut row = json!({
        "key": format!("failed:{}", q.id),
        "kind": "failed",
        "question_id": q.id,
        "group": q.group(),
        "header": q.header,
        "question": question,
        "asked_by": q.asked_by,
        "answer": answer,
        "answered_at": q.answered_at.as_deref().unwrap_or(&q.created_at),
        "retries": store.retry_count(q.id)?,
        "cut": cut,
        "asker": asker::of_question(store, q)?,
        "delivery": delivery(store, q, auto_retry)?,
    });
    let published = token(&row);
    row["actions"] = json!(ACTIONS);
    row["token"] = json!(published);
    Ok(row)
}

fn delivery(store: &Store, q: &Question, auto_retry: bool) -> rusqlite::Result<Value> {
    let (state, reason) = state(q.delivery_error.as_deref());
    let wait = (state == "pending").then(|| wait(reason.unwrap_or_default()));
    Ok(json!({
        "state": state,
        "reason": reason,
        "wait": wait,
        "attempts": q.delivery_attempts,
        // a stranded row keeps its clock in the database; to the feed it waits for nobody
        "pending_since": if state == "pending" { store.pending_since(q.id)? } else { None },
        "auto_retry": auto_retry,
    }))
}

/// `state` and `reason` of a stored `delivery_error`. A claim's token is never
/// published.
fn state(stored: Option<&str>) -> (&'static str, Option<&str>) {
    let Some(stored) = stored else {
        return ("unattempted", None);
    };
    if stored.starts_with("sending:") {
        return ("claimed", None);
    }
    if let Some(reason) = stored.strip_prefix(PENDING) {
        return ("pending", Some(reason));
    }
    if let Some(reason) = stored.strip_prefix(AGENT_GONE) {
        return ("agent_gone", Some(reason));
    }
    ("failed", Some(stored))
}

/// The first row of the document's table that fits.
fn wait(reason: &str) -> &'static str {
    if reason == BUSY_WAIT_IDLE {
        "busy_idle"
    } else if reason.starts_with("requeued:") {
        "requeued"
    } else if reason == PANEL {
        "panel"
    } else if reason.starts_with("busy:") {
        "busy"
    } else if reason.starts_with("blocked:") {
        "blocked"
    } else if reason == OPERATOR_IN_PANE {
        "operator_here"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests;

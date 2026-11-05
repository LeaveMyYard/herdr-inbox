//! The caller's own questions: `list_my_questions` and `withdraw_question`
//! (Python `server.list_my_questions`, `withdraw_question`, `Question.brief`,
//! `service.close`).

use serde_json::{Value, json};

use crate::claude::Transcripts;
use crate::mcp::caller::Caller;
use crate::remote::pyvalue::py_truthy;
use crate::store::alerts::Effect;
use crate::store::{Question, Store};

/// What an agent gets back about its own question (Python `Question.brief`).
fn brief(q: &Question) -> Value {
    let mut out = json!({"id": q.id, "status": q.status, "question": q.question});
    if q.status == "answered" {
        out["answer"] = json!(q.answer);
        if let Some(detail) = q.answer_json.as_ref().filter(|d| py_truthy(d)) {
            out["answer_detail"] = detail.clone();
        }
    }
    out
}

/// `list_my_questions`: the caller's questions (open and answered, or all) and
/// its alerts (open, or all).
pub fn list_my_questions(
    store: &Store,
    caller: &Caller,
    include_closed: bool,
    files: &Transcripts,
) -> rusqlite::Result<Value> {
    let questions = store.mine(caller.owner(), include_closed, files)?;
    let alerts: Vec<Value> = store
        .my_alerts(caller.owner(), files)?
        .into_iter()
        .filter(|a| include_closed || a.status == "open")
        .map(|a| json!({"id": a.reference(), "title": a.title, "severity": a.severity, "status": a.status}))
        .collect();
    let mut out = json!({"questions": questions.iter().map(brief).collect::<Vec<_>>()});
    if !alerts.is_empty() {
        out["alerts"] = Value::Array(alerts);
    }
    Ok(out)
}

/// `withdraw_question`: the caller's open question `qid` closed as withdrawn
/// with `reason`; the receipt, and the effects (the pane's markers).
pub fn withdraw_question(
    store: &Store,
    caller: &Caller,
    qid: i64,
    reason: &str,
    files: &Transcripts,
) -> rusqlite::Result<(Value, Vec<Effect>)> {
    let mine = store.mine(caller.owner(), true, files)?;
    let Some(q) = mine.iter().find(|q| q.id == qid) else {
        let msg = format!("Q{qid} is not a question of this session.");
        return Ok((
            json!({"id": qid, "status": "error", "message": msg}),
            Vec::new(),
        ));
    };
    // Python `service.close`: re-read, then close only an open one
    let pane = store.get(qid)?.and_then(|q| q.pane_id);
    if !store.set_status(qid, "withdrawn", Some(reason))? {
        let msg = format!("Q{qid} is {}, not open.", q.status);
        return Ok((
            json!({"id": qid, "status": "error", "message": msg}),
            Vec::new(),
        ));
    }
    let receipt =
        json!({"id": qid, "status": "withdrawn", "message": format!("Q{qid} withdrawn.")});
    Ok((receipt, vec![Effect::Markers(pane)]))
}

#[cfg(test)]
mod tests;

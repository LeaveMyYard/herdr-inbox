//! `check_answers` (Python `server.check_answers`): the caller's answers it
//! has not had yet, marked seen, and its alerts the operator acknowledged
//! since the last report.
//!
//! Marking an answer seen is what keeps it from being typed as well: a
//! delivery claims only answers its agent has not fetched
//! (`Store::claim_delivery`). An answer whose delivery is in flight (claimed
//! before this call) is returned all the same and its claim kept, as in
//! Python: the typing that holds it may still land it. The operator kept
//! this on 2026-11-05 (docs/DESIGN.md, "Rust port decisions").
//!
//! A stated deviation: Python runs the three steps in autocommit, so a
//! failure after the answers were marked seen (the alerts' write, busy) lost
//! them, and two parallel calls could both return them. Here they run in one
//! write transaction.

use serde_json::{Value, json};

use crate::claude::Transcripts;
use crate::mcp::caller::Caller;
use crate::store::Store;

/// The result the agent reads.
pub fn check_answers(
    store: &Store,
    caller: &Caller,
    files: &Transcripts,
) -> rusqlite::Result<Value> {
    let owner = caller.owner();
    // one write transaction: an answer is marked seen only together with the result that
    // returns it (a failed alert write rolls it back for the retry), and two parallel calls
    // never both return it
    let (answers, acked) = store.immediate(|_| {
        let answers = store.unacknowledged_answers(owner, files)?;
        let ids: Vec<i64> = answers.iter().map(|q| q.id).collect();
        store.acknowledge(&ids)?;
        Ok((answers, store.acknowledged_unreported(owner, files)?))
    })?;
    let mut out = json!({"answers": answers
        .iter()
        .map(|q| json!({"id": q.id, "question": q.question, "answer": q.answer}))
        .collect::<Vec<_>>()});
    if !acked.is_empty() {
        out["alerts_acknowledged"] = acked
            .iter()
            .map(|a| json!({"id": a.reference(), "title": a.title}))
            .collect();
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

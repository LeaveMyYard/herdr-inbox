//! `alert_operator` and `withdraw_alert` (Python `server.alert_operator`,
//! `withdraw_alert`, `alerts.raise_alert`, `alerts.withdraw`): the store side
//! is `store::alerts`; here are the receipts the agent reads.

use serde_json::{Value, json};

use crate::claude::Transcripts;
use crate::mcp::caller::Caller;
use crate::mcp::receipts;
use crate::store::Store;
use crate::store::alerts::{Asker, Effect, RaiseOutcome, WithdrawOutcome};

/// What `alert_operator` asks for.
pub struct Raise<'a> {
    pub title: &'a str,
    pub message: Option<&'a str>,
    pub severity: &'a str,
    pub asked_by: Option<&'a str>,
}

/// `alert_operator`: the receipt and the effects (a notification, the markers).
pub fn alert_operator(
    store: &Store,
    caller: &Caller,
    raise: &Raise<'_>,
    server: &str,
    files: &Transcripts,
) -> rusqlite::Result<(Value, Vec<Effect>)> {
    let asker = Asker {
        owner: caller.owner(),
        tab_id: caller.tab_id.as_deref(),
        tab_label: caller.tab_label.as_deref(),
    };
    let outcome = store.raise_alert(
        asker,
        raise.title,
        raise.message,
        raise.severity,
        raise.asked_by,
        server,
        files,
    )?;
    Ok(match outcome {
        RaiseOutcome::Unavailable => (receipts::unavailable(), Vec::new()),
        RaiseOutcome::NoTitle => (receipts::error("An alert needs a short title."), Vec::new()),
        RaiseOutcome::AlreadyOpen(id) => (
            json!({"id": id, "status": "already_open",
                   "message": format!("A{id} is already with the operator; do not raise it again.")}),
            Vec::new(),
        ),
        RaiseOutcome::Raised(id, effects) => (
            json!({"id": id, "status": "open", "message": format!(
                "Raised as A{id}. The operator sees it in the inbox and will check; no answer comes back. \
                 check_answers reports when it is acknowledged. Withdraw it (withdraw_alert) if it resolves itself.")}),
            effects,
        ),
    })
}

/// `withdraw_alert`: the receipt and the effects (the markers).
pub fn withdraw_alert(
    store: &Store,
    caller: &Caller,
    aid: i64,
    files: &Transcripts,
) -> rusqlite::Result<(Value, Vec<Effect>)> {
    Ok(match store.withdraw_alert(caller.owner(), aid, files)? {
        WithdrawOutcome::NotYours => (
            json!({"id": aid, "status": "error", "message": format!("A{aid} is not an alert of this session.")}),
            Vec::new(),
        ),
        WithdrawOutcome::NotOpen(status) => (
            json!({"id": aid, "status": "error", "message": format!("A{aid} is {status}, not open.")}),
            Vec::new(),
        ),
        WithdrawOutcome::Withdrawn(effects) => (
            json!({"id": aid, "status": "withdrawn", "message": format!("A{aid} withdrawn.")}),
            effects,
        ),
    })
}

#[cfg(test)]
mod tests;

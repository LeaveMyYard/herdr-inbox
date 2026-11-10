//! An `alert` row.

use serde_json::{Value, json};

use super::asker;
use super::token::token;
use crate::store::Store;
use crate::store::alerts::Alert;

/// What the row takes.
pub const ACTIONS: [&str; 1] = ["ack"];

/// The row of alert `a`.
pub fn entry(store: &Store, a: &Alert) -> rusqlite::Result<Value> {
    let mut row = json!({
        "key": format!("alert:{}", a.id),
        "kind": "alert",
        "id": a.id,
        "alert_kind": a.kind,
        "severity": a.severity,
        "title": a.title,
        "message": a.message,
        "asked_by": a.asked_by,
        "created_at": a.created_at,
        "asker": asker::of_alert(store, a)?,
    });
    let published = token(&row);
    row["actions"] = json!(ACTIONS);
    row["token"] = json!(published);
    Ok(row)
}

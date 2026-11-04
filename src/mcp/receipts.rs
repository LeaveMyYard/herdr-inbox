//! The receipts a tool returns when it cannot do its work (Python
//! `service.UNAVAILABLE`, `service.NOT_MIGRATED` and `busy_safe`'s busy
//! answer), word for word: agents act on this text.

use serde_json::{Value, json};

/// The inbox serves only the top-level agent of a herdr pane.
pub const UNAVAILABLE: &str = "The operator inbox only serves the top-level agent of a herdr pane, and this \
session is not one (not in herdr, or nested inside another agent). Ask in chat, or ask \
your caller, instead.";

/// The database is not migrated to this code's schema yet.
pub const NOT_MIGRATED: &str = "The operator inbox is being upgraded (its delivery worker migrates the database); \
call this tool again in a few seconds, or ask in chat if it stays unavailable.";

/// The database stayed busy through every retry.
pub const BUSY: &str =
    "The inbox database is busy right now; call this tool again in a few seconds.";

/// The `unavailable` receipt of a session outside herdr (or nested in an agent).
pub fn unavailable() -> Value {
    json!({"status": "unavailable", "message": UNAVAILABLE})
}

/// The `unavailable` receipt of a database the worker has not migrated yet.
pub fn not_migrated() -> Value {
    json!({"status": "unavailable", "message": NOT_MIGRATED})
}

/// The answer to a call the database stayed too busy for.
pub fn busy() -> Value {
    json!({"status": "error", "message": BUSY})
}

/// An error receipt (not a tool error): the call was understood and refused.
pub fn error(message: impl Into<String>) -> Value {
    json!({"status": "error", "message": message.into()})
}

//! The receipts a tool returns when it cannot do its work (Python
//! `service.UNAVAILABLE`, `service.NOT_MIGRATED` and `busy_safe`'s busy
//! answer), word for word: agents act on this text.

use serde_json::{Value, json};

/// The inbox serves only the top-level agent of a herdr pane.
pub const UNAVAILABLE: &str = "The operator inbox only serves the top-level agent of a herdr pane, and this \
session is not one (not in herdr, or nested inside another agent). Ask in chat, or ask \
your caller, instead.";

/// This terminal could not be confirmed just now. Not [`UNAVAILABLE`]: an agent told to
/// give up here would abandon an answered question over one slow read.
pub const UNDECIDED: &str = "The operator inbox could not confirm this terminal just now (herdr-core was slow, \
or the terminal is still starting). Your questions and any answers are kept. Call this tool again in a \
few seconds, and ask in chat only if it stays unavailable.";

/// This session is refused in its core terminal for a reason that stays. Not
/// [`UNAVAILABLE`], which says "not in herdr": here that would be untrue.
pub const UNPROVED: &str = "The operator inbox could not confirm this session as the top-level agent of its \
herdr-app terminal: it may be nested inside another agent, or started through a wrapper. Ask in chat, or \
ask your caller, instead.";

/// A second live process runs this conversation. It lasts until one of them ends, so calling
/// again soon is no use, and only the operator or the agent can end it.
pub const SHARED: &str = "This conversation is also running in another live Claude Code process (it was \
resumed elsewhere), so the operator inbox cannot tell which copy is asking. Your questions and any answers \
are kept. Calling again will not help while both run: close one copy, then call this tool again. Until \
then, ask in chat.";

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

/// The `unavailable` receipt of a session refused in its core terminal.
pub fn unproved() -> Value {
    json!({"status": "unavailable", "message": UNPROVED})
}

/// The `unavailable` receipt of a conversation two live processes run.
pub fn shared() -> Value {
    json!({"status": "unavailable", "message": SHARED})
}

/// The `unavailable` receipt of a moment this terminal could not be confirmed in.
pub fn undecided() -> Value {
    json!({"status": "unavailable", "message": UNDECIDED})
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

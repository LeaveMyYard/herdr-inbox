//! The texts a delivery leaves in `delivery_error` (Python `service.py`'s
//! constants), byte for byte: the popup, the CLI and `check_answers` show
//! them, and the Python processes still in the field read them.

/// No pane known: the answer waits for `check_answers`.
pub const NO_PANE: &str = "no pane: answer available through check_answers";
/// Prefix of "no live agent in pane <pane>".
pub const NO_AGENT: &str = "no live agent in pane";
/// Prefix of "conversation changed in pane <pane>".
pub const CHANGED: &str = "conversation changed in pane";
/// The agent is at a prompt or dialog.
pub const BLOCKED_WAIT: &str = "blocked: the agent is at a prompt or dialog";
/// Left to the idle session's rewake watcher.
pub const WATCHER_WAIT: &str = "idle: the session's own hook wakes it with the answer";
/// Left to the working session's own hook.
pub const HOOK_WAIT: &str = "busy: delivered at the agent's next tool call or turn end";
/// The `delivery_error` of an answer requeued after an old server's lost delivery.
pub const REQUEUED: &str = "pending: requeued: an earlier delivery never reached the conversation; sending when the agent is idle";
/// [`REQUEUED`] without its `pending: ` (what `set_pending` is given).
pub const REQUEUED_WAIT: &str =
    "requeued: an earlier delivery never reached the conversation; sending when the agent is idle";
/// Typed while busy, never seen: now only at rest.
pub const BUSY_WAIT_IDLE: &str =
    "busy: typed while the agent was busy but never seen; waiting for it to be idle";
/// The operator may be typing there.
pub const OPERATOR_IN_PANE: &str = "waiting: the operator is in that pane";
/// The background-agents panel has focus: typed text never reaches the conversation.
pub const PANEL: &str = "blocked: panel: the agent's background-task panel has focus";
/// Unsent text in Claude Code's input box.
pub const COMPOSER_DRAFT: &str =
    "waiting: the agent's input box holds unsent text (send or clear it there)";
/// Claude Code's input box cannot be read.
pub const COMPOSER_UNKNOWN: &str = "waiting: cannot read the agent's input box";
/// Typed, accepted by herdr, never seen in the conversation.
pub const NOT_VERIFIED: &str =
    "not verified: herdr accepted the text but it never reached the conversation";
const OLDER: &str = "older conversation: ";

/// The state of an answer whose pane runs (or got it typed into) an older
/// session of its conversation (Python `older_error`).
pub fn older_error(pane: &str, live: Option<&str>, asker: Option<&str>, typed: bool) -> String {
    let short = |s: Option<&str>| s.unwrap_or("?").chars().take(8).collect::<String>();
    let (old, ask) = (short(live), short(asker));
    if typed {
        format!(
            "{OLDER}typed into {old} in pane {pane}, an older session than the asking {ask}, which never got it; resume {ask} to deliver"
        )
    } else {
        format!(
            "{OLDER}pane {pane} runs {old}, an older session than the asking {ask}; resume {ask} to deliver"
        )
    }
}

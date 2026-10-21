//! What one pass of the delivery worker does with a pane's waiting answers
//! (Python `service.deliver_batch`), as a pure decision: the caller looks
//! things up through [`Look`] when the decision asks (in Python's order, so
//! an answer that is settled early costs no further lookup) and carries out
//! the returned [`Outcome`]s. Typing the prompt and waiting for it to land are
//! the caller's too ([`Look::prompt`], [`Look::landing`]).
//!
//! Every rule reproduces the `deliver_batch` rows of the delivery decision
//! table (`tests/fixtures/decisions.json`; `tests/deliver_rows.rs`).

mod decide;
mod reasons;
mod typing;

pub use decide::decide;
pub use reasons::{
    BLOCKED_WAIT, BUSY_WAIT_IDLE, CHANGED, COMPOSER_DRAFT, COMPOSER_UNKNOWN, HOOK_WAIT, NO_AGENT,
    NO_PANE, NOT_VERIFIED, OPERATOR_IN_PANE, PANEL, REQUEUED, REQUEUED_WAIT, WATCHER_WAIT,
    older_error,
};

/// Busy-while-typing misses after which an answer waits for idle (Python
/// `MAX_BUSY_ATTEMPTS`).
pub const MAX_BUSY_ATTEMPTS: i64 = 1;
/// Unseen hook sends after which an answer is left to typing (Python
/// `store.MAX_HOOK_MISSES`).
pub const MAX_HOOK_MISSES: i64 = 2;

/// How this pass was asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    /// The worker's retry loop (not a fresh answer).
    pub retry: bool,
    /// The operator's "Send now": no busy grace, no focused-pane wait.
    pub force: bool,
    /// Typed now, verified by a detached process (the popup).
    pub verify_later: bool,
}

/// An answer waiting in the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    /// Its question id.
    pub id: i64,
    /// The asking session, if known.
    pub session: Option<String>,
    /// Requeued after an old server's lost delivery (`delivery_error` is [`REQUEUED`]).
    pub requeued: bool,
}

/// The pane's agent as herdr reports it (Python `herdr.live_agent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    /// Its kind (`claude`, `codex`, ...).
    pub kind: Option<String>,
    /// `idle`, `done`, `working`, `blocked`, ...
    pub status: String,
    /// The operator is in the pane.
    pub focused: bool,
    /// The conversation it runs (the followed session).
    pub session: Option<String>,
    /// The session herdr reports (it differs after herdr resumed an older id).
    pub reported_session: Option<String>,
}

/// Claude Code's input box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composer {
    /// Nothing typed: the prompt can go.
    Empty,
    /// Unsent text: typing would be appended to it.
    Draft,
    /// Unreadable: counts as not empty.
    Unknown,
}

/// Where a typed answer landed (Python `_record_outcome`'s hits and plan).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Landing {
    /// In a transcript of the asker's forward family: delivered.
    pub counted: bool,
    /// Anywhere watched.
    pub anywhere: bool,
    /// The row's state if it landed only in an older session of its conversation.
    pub older_error: Option<String>,
}

/// What becomes of one answer (the store write Python makes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not claimed (another process has it): untouched.
    Untouched,
    /// A failed attempt (`record_delivery(error)`; its `acknowledge` does not
    /// apply to an error): the error text.
    Failed(String),
    /// Delivered and found in the conversation (`record_delivery(None,
    /// verified=True)`).
    Delivered {
        /// Counted as seen: false when only the long-answer reference landed
        /// (the agent still has to call `check_answers`).
        ack: bool,
    },
    /// Typed, with no transcript to check it against: delivered, never
    /// counted as seen, so `check_answers` still returns it (never retried:
    /// that could double it).
    SentUnverified,
    /// Not sent yet (`set_pending`): the reason, without `pending: `.
    Pending(String),
    /// Typed while busy, never seen (`record_busy_miss`): waits for idle.
    BusyMiss,
    /// Its conversation runs in another pane now (`move_to_pane`): delivered there next.
    Moved,
    /// Typed; a detached `inbox verify` records the outcome under the claim.
    VerifyLater,
}

/// The lookups a decision may make, each only when it needs it.
pub trait Look {
    /// Claims the answer (another process may hold it).
    fn claimed(&mut self, id: i64) -> bool;
    /// The pane's live agent.
    fn agent(&mut self) -> Option<Agent>;
    /// Its conversation runs in another pane (and it was moved there).
    fn moved_elsewhere(&mut self, id: i64) -> bool;
    /// The pane runs a conversation outside the asker's forward family
    /// (never for an answer whose asking session is unknown: Python's
    /// `q.session_id and ...`, recorded as one observation).
    fn conversation_changed(&mut self, id: i64) -> bool;
    /// That conversation is an older session of the asker's.
    fn pane_runs_older_session(&mut self, id: i64) -> bool;
    /// The idle session's rewake watcher is alive.
    fn watched(&mut self) -> bool;
    /// Unseen hook sends of the answer.
    fn hook_misses(&mut self, id: i64) -> i64;
    /// The working session's hook fired recently.
    fn hook_live(&mut self) -> bool;
    /// Busy-while-typing misses of the answer.
    fn busy_attempts(&mut self, id: i64) -> i64;
    /// Pending for less than the busy grace.
    fn within_busy_grace(&mut self, id: i64) -> bool;
    /// Claude Code's background-agents panel has focus.
    fn panel_focused(&mut self) -> bool;
    /// Already in the conversation: the expected segment indexes found, per answer.
    fn landed(&mut self, ids: &[i64]) -> Vec<Option<Vec<usize>>>;
    /// The pane reports a session other than the followed one.
    fn pane_reports_another_session(&mut self) -> bool;
    /// Already typed into that older session.
    fn already_typed_into_older_session(&mut self, id: i64) -> bool;
    /// The messages the answers go in: (ids, the full answer is in it).
    fn messages(&mut self, ids: &[i64]) -> Vec<(Vec<i64>, bool)>;
    /// The agent looked up again just before typing.
    fn agent_again(&mut self, ids: &[i64]) -> Option<Agent>;
    /// Claude Code's input box.
    ///
    /// Two notifications are the caller's, not an [`Outcome`] (Python
    /// `deliver_batch`): under "Send now", an input box that is not empty
    /// notifies `"{refs} not sent"` with the pending reason minus
    /// `waiting: `, once per message; and an input box that stays
    /// unreadable notifies once per pane (`_note_unreadable`).
    fn composer(&mut self, ids: &[i64]) -> Composer;
    /// Types the message: herdr's error, if any (an empty one counts as
    /// none, as Python's `if err:`).
    fn prompt(&mut self, ids: &[i64]) -> Option<String>;
    /// A transcript to verify against is known.
    fn verify_transcript(&mut self, ids: &[i64]) -> bool;
    /// Waits for the typed message and reports where it landed.
    fn landing(&mut self, id: i64) -> Landing;
}

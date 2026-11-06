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

pub use crate::verify::Landing;
pub use decide::decide;
pub use reasons::{
    BLOCKED_WAIT, BUSY_WAIT_IDLE, CHANGED, COMPOSER_DRAFT, COMPOSER_UNKNOWN, HOOK_WAIT, NO_AGENT,
    NO_PANE, NOT_IDLE, NOT_VERIFIED, OLDER, OPERATOR_IN_PANE, PANEL, REQUEUED, REQUEUED_WAIT,
    TERMINAL_UNREAD, WATCHER_WAIT, not_idle, older_error, terminal_unread,
};

/// Busy-while-typing misses after which an answer waits for idle (Python
/// `MAX_BUSY_ATTEMPTS`).
pub const MAX_BUSY_ATTEMPTS: i64 = 1;
/// Unseen hook sends after which an answer is left to typing (Python
/// `store.MAX_HOOK_MISSES`).
pub const MAX_HOOK_MISSES: i64 = 2;
/// "No agent" / "conversation changed" tries after which the agent counts as
/// gone (Python `service.MAX_STALE_RETRIES`).
pub const MAX_STALE_RETRIES: i64 = 10;
/// Typed-but-never-seen tries after which an answer is no longer typed
/// (Python `service.MAX_UNVERIFIED_ATTEMPTS`).
pub const MAX_UNVERIFIED_ATTEMPTS: i64 = 2;

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
    /// `idle`, `done`, `working`, `blocked`, ... A status herdr does not
    /// report is `"None"` here, as Python renders it (`busy: the agent is None`).
    pub status: String,
    /// The operator is in the pane.
    pub focused: bool,
    /// The conversation it runs (the followed session).
    pub session: Option<String>,
    /// The session herdr reports (it differs after herdr resumed an older id).
    pub reported_session: Option<String>,
}

impl Agent {
    /// herdr reports Claude Code in the pane (its kind `claude`): an input
    /// box that cannot be read then holds the answer. Any other kind is
    /// still held by a recognised Claude Code draft (#96).
    pub fn is_claude(&self) -> bool {
        self.kind.as_deref() == Some("claude")
    }

    /// The pending reason when its input box in `state` holds the answer
    /// (a draft, for any kind; an unreadable box, for Claude only), or
    /// `None` when it can be typed into. The one rule for the hold and for
    /// Send now's "not sent" notice.
    pub fn held_by(&self, state: Composer) -> Option<&'static str> {
        match state {
            Composer::Empty => None,
            Composer::Draft => Some(COMPOSER_DRAFT),
            Composer::Unknown if self.is_claude() => Some(COMPOSER_UNKNOWN),
            Composer::Unknown => None,
        }
    }
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

/// What became of typing a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompted {
    /// It went, or may have: it waits for the conversation to show it.
    Sent,
    /// It failed, a failed attempt: the error text.
    Failed(String),
    /// The host refused it, and nothing was typed: no attempt, it waits for
    /// this reason (a pending reason, without `pending: `).
    Refused(String),
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
    /// Its job needs what this does not carry: the message's full-answer flag
    /// (from [`Look::messages`]) and whether the pane was busy (its agent's
    /// status not `idle` or `done`), Python's `"ack"` and `"busy"`; a job
    /// read without them is `ack = true`, `busy = false` (Python
    /// `job.get("ack", True)`, `job.get("busy")`). The job's outcome is
    /// [`crate::verify::job`].
    VerifyLater,
}

/// The lookups a decision may make, each only when it needs it.
pub trait Look {
    /// Claims the answer (another process may hold it).
    fn claimed(&mut self, id: i64) -> bool;
    /// The pane's live agent.
    fn agent(&mut self) -> Option<Agent>;
    /// [`Look::agent`] found none because it could not look: why, as a
    /// pending reason. No proof that the agent is gone, so no try is counted.
    /// herdr has no such answer: a pane it cannot read is a pane with no agent.
    fn agent_unread(&mut self) -> Option<String> {
        None
    }
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
    /// Claude Code's input box, read before every message for any agent
    /// kind (#96).
    ///
    /// Two notifications are the caller's, not an [`Outcome`] (Python
    /// `deliver_batch`): under "Send now", an input box that holds the
    /// answer (a draft; for a [`Agent::is_claude`] agent also an unreadable
    /// box) notifies `"{refs} not sent"` with the pending reason minus
    /// `waiting: `, once per message; and a Claude agent's input box that
    /// stays unreadable notifies once per pane (`_note_unreadable`).
    fn composer(&mut self, ids: &[i64]) -> Composer;
    /// Types the message (an empty [`Prompted::Failed`] counts as sent, as
    /// Python's `if err:`).
    fn prompt(&mut self, ids: &[i64]) -> Prompted;
    /// A transcript to verify against is known.
    fn verify_transcript(&mut self, ids: &[i64]) -> bool;
    /// Waits for the typed message and reports where it landed.
    fn landing(&mut self, id: i64) -> Landing;
    /// An answer's outcome, the moment it is decided: before the next lookup
    /// and before the next message is typed (Python writes it there). The
    /// caller writes it to the store, or hands a [`Outcome::VerifyLater`]
    /// job over. Each answer's outcome comes once; [`Outcome::Untouched`]
    /// never comes.
    ///
    /// A caller whose own work fails (its database) must type nothing more
    /// from then on: [`Look::prompt`] refuses, so no answer is typed whose
    /// outcome would not be written (Python leaves the pass at the failing
    /// statement).
    fn record(&mut self, id: i64, outcome: &Outcome);
}

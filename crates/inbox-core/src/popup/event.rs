//! What goes into the popup model ([`Open`], [`Event`]) and what comes out
//! ([`Effect`]): the model does no IO, the caller carries the effects out
//! (database writes, herdr calls, toasts, closing the popup) and reports
//! back with the next events.

use super::data::{Detail, Snapshot};
use super::entry::Entry;
use super::key::Key;

/// How the popup opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Open {
    /// The operator's pane (its groups head the list).
    pub focus_pane: Option<String>,
    /// The database now.
    pub snapshot: Snapshot,
    /// `inbox open --question …` from another plugin (Python `deeplink`).
    pub deeplink: Option<DeepLink>,
}

/// A deep link, resolved by the IO layer (Python `deeplink.resolve`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeepLink {
    /// The group (or ⚠ entry) it names, if still open.
    pub target: Option<Entry>,
    /// The question to show first.
    pub qid: Option<i64>,
    /// Hold every answering key until a deliberate Tab/→ (`--no-hold` is
    /// false: an operator key opened it).
    pub hold: bool,
    /// The notice when the target is gone (`Q4 is no longer open`).
    pub missing: String,
}

/// Something that happened, at the time passed with it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// The operator pressed a key.
    Key(Key),
    /// The periodic poll read the database (Python `InboxApp.poll`).
    Poll(Snapshot),
    /// A [`Send`] was recorded (the database as it is now); delivery was
    /// handed off (Python `_after_submit`, detached).
    Sent {
        /// The questions it answered (or retried).
        done: Vec<i64>,
        /// How many it was asked to send.
        wanted: usize,
        /// The database after it.
        snapshot: Snapshot,
    },
    /// A [`Effect::GoTo`] finished (Python `_focused`).
    Focused {
        /// The pane.
        pane: String,
        /// herdr's error, if focusing failed.
        error: Option<String>,
    },
}

/// A toast's severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Information.
    Information,
    /// A warning.
    Warning,
}

impl Severity {
    /// The golden's name (Textual's).
    pub fn name(self) -> &'static str {
        match self {
            Severity::Information => "information",
            Severity::Warning => "warning",
        }
    }
}

/// An answer to record and hand to delivery. The caller records it in its
/// own thread, then reports [`Event::Sent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Send {
    /// A lone question, answered and sent at once (Python `service.answer`).
    Answer {
        /// The question.
        qid: i64,
        /// The answer (`None`: nothing to send, the call records nothing).
        detail: Option<Detail>,
    },
    /// A group's drafted answers among `qids` (Python `service.submit`).
    Submit {
        /// The questions.
        qids: Vec<i64>,
    },
    /// An undelivered answer's Retry / Send now: delivered again with
    /// force, unless another process is sending it right now.
    Retry {
        /// The question.
        qid: i64,
    },
}

/// What the caller does for the model, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Effect {
    /// Show a toast.
    Notify {
        /// Its text.
        text: String,
        /// Its severity.
        severity: Severity,
    },
    /// Store (or with `None` clear) a draft (Python `store.set_draft`).
    Draft {
        /// The question.
        qid: i64,
        /// The draft.
        detail: Option<Detail>,
    },
    /// Record answers and hand them to delivery (Python `_send`).
    Send(Send),
    /// Dismiss an open question (Python `service.close(…, "dismissed")`).
    Dismiss {
        /// The question.
        qid: i64,
    },
    /// Stop showing an undelivered answer (Python `store.hide_failed`).
    Hide {
        /// The question.
        qid: i64,
    },
    /// Acknowledge an alert (Python `alerts.acknowledge`).
    Ack {
        /// The alert.
        alert: i64,
    },
    /// Go to an agent's pane, then report [`Event::Focused`]. Not `graced`:
    /// the caller first asks herdr whether that agent waits at a prompt, and
    /// if it does only says "That agent is waiting at a prompt: pause, then
    /// press g" (Python `_focus_pane`).
    GoTo {
        /// The pane.
        pane: String,
        /// The `g` came after a pause (Python `go_graced`).
        graced: bool,
    },
    /// Close the popup now.
    Exit,
    /// Close the popup once every in-flight [`Effect::Send`] is recorded
    /// (Python `action_quit`).
    Quit,
}

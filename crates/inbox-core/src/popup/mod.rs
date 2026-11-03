//! The inbox popup as a pure state machine (Python `tui.InboxApp`, plan
//! herdr-app-inbox §2.3 P6-A): keys, polls and IO results in, effects out.
//! It decides which answer goes to which question, so every rule here is
//! the Python popup's, replayed from the transition golden
//! `tests/fixtures/popup-transitions.json` (`tests/popup_transitions.rs`;
//! the herdr-app mod's JavaScript model replays the same file).
//!
//! The rule above all: **no background event changes what the next key
//! does.**
//!
//! - Cursors are targets ([`Entry`], [`Tab`], a review row's question),
//!   never indexes: rows coming or going under a cursor never move it.
//! - Arrivals are appended to the list; a row that arrived less than
//!   [`ROW_ARM`] ago takes no Enter and no digit.
//! - A row or a tab whose question closed stays in place as an inert
//!   placeholder until the operator moves.
//! - Nothing joins an open group; whether Enter sends at once (a lone
//!   question) is fixed when the group opens.
//! - Every view ignores Enter, digits and Space for [`VIEW_ARM`] after it
//!   appears; after a view change the operator did not cause, every key is
//!   dropped for [`GRACE`].
//! - A deep-linked popup holds every answering key until a deliberate Tab/→
//!   at least [`RELEASE`] after it opened; `g` to an agent waiting at a
//!   prompt (every alert) needs a [`GO_PAUSE`] after the previous key.
//!
//! The model does no IO and reads no clock: the caller passes the time with
//! every event and carries out the returned [`Effect`]s.

mod actions;
mod data;
mod entry;
mod event;
mod group;
mod key;
mod keys;
mod list;
mod ordered;
mod poll;
mod question;
mod review;
mod rows;
mod state;
mod textbox;
mod views;

pub use data::{AGENT_GONE, Alert, Detail, Millis, QOption, Question, Snapshot};
pub use entry::{Entry, Mode, Tab};
pub use event::{DeepLink, Effect, Event, Open, Send, Severity};
pub use key::Key;
pub use ordered::Ordered;
pub use question::{Editing, QuestionView, Row};
pub use review::{ReviewCursor, ReviewView};
pub use state::{Confirm, Popup, Timers};
pub use textbox::TextBox;
pub use views::{AlertView, FailedView, GoneView, ListView, View};

/// A view ignores Enter, digits and Space this long after it appears
/// (Python `ARM_SECONDS`): a key already on its way never answers a
/// question the operator has not seen.
pub const VIEW_ARM: Millis = 400;
/// A list row that arrived while the popup was open takes no Enter and no
/// digit this long (Python `ROW_ARM_SECONDS`).
pub const ROW_ARM: Millis = 1000;
/// After a view change the operator did not cause, keys are dropped this
/// long (Python `GRACE_SECONDS`).
pub const GRACE: Millis = 500;
/// A held deep link is released only by Tab/→ this long after the popup
/// opened (Python `RELEASE_GRACE`).
pub const RELEASE: Millis = 500;
/// `g` to a prompt-waiting agent needs this pause after the previous key
/// and the last view change (Python `ALERT_GO_GRACE`).
pub const GO_PAUSE: Millis = 500;

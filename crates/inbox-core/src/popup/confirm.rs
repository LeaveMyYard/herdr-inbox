//! "Dismiss … without answering?" (Python `ConfirmDismiss`): what it names
//! and what `y` dismisses.

use super::entry::Entry;
use super::remote::RemoteQuestion;

/// "Dismiss … without answering?": only `y` confirms, any other key keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    /// What it names: `Q7`, `Q2–Q4 (3 questions)`, or a remote group's
    /// `project1:Q3`.
    pub what: String,
    pub(super) target: ConfirmTarget,
}

impl Confirm {
    /// The local questions `y` dismisses (the renderer shows the first one's
    /// text); none for a remote group ([`Confirm::remote`]).
    pub fn qids(&self) -> &[i64] {
        match &self.target {
            ConfirmTarget::Question(q) => std::slice::from_ref(q),
            ConfirmTarget::Group(_, qids) => qids,
            ConfirmTarget::Remote { .. } => &[],
        }
    }

    /// The list row it dismisses as a whole, if it is a whole group.
    pub fn entry(&self) -> Option<&Entry> {
        match &self.target {
            ConfirmTarget::Question(_) => None,
            ConfirmTarget::Group(entry, _) | ConfirmTarget::Remote { entry, .. } => Some(entry),
        }
    }

    /// The remote questions `y` asks their machine to dismiss, as they were
    /// when `d` was pressed.
    pub fn remote(&self) -> Option<&[RemoteQuestion]> {
        match &self.target {
            ConfirmTarget::Remote { questions, .. } => Some(questions),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConfirmTarget {
    /// One question of the open group.
    Question(i64),
    /// A list row's whole group (its open questions when `d` was pressed).
    Group(Entry, Vec<i64>),
    /// A remote group's row: its open questions when `d` was pressed, and
    /// its machine's name.
    Remote {
        entry: Entry,
        label: String,
        questions: Vec<RemoteQuestion>,
    },
}

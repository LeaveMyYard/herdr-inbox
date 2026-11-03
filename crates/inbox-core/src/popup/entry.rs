//! The names of the popup's rows, tabs and modes. A cursor is always one of
//! these names (a target), never an index: rows coming or going under it can
//! never move it onto something else.

use std::fmt;

/// A row of the group list (Python `Entry`, `("group", id)` ...). Its
/// [`Display`](fmt::Display) form is the golden's (`group:2`, `failed:5`,
/// `gone`, `alert:1`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Entry {
    /// One ask_operator call's open questions (the group id).
    Group(i64),
    /// An answer that was not delivered (its question id).
    Failed(i64),
    /// The one summary row of the answers whose agents are gone (Q167).
    Gone,
    /// An alert (its id).
    Alert(i64),
}

impl Entry {
    /// Parse the golden's form (`group:2`, `gone`, ...).
    pub fn parse(s: &str) -> Option<Entry> {
        if s == "gone" {
            return Some(Entry::Gone);
        }
        let (kind, key) = s.split_once(':')?;
        let n: i64 = key.parse().ok()?;
        match kind {
            "group" => Some(Entry::Group(n)),
            "failed" => Some(Entry::Failed(n)),
            "alert" => Some(Entry::Alert(n)),
            _ => None,
        }
    }

    /// An alert row: unnumbered, never where the cursor starts (Python
    /// `is_alert`).
    pub fn is_alert(&self) -> bool {
        matches!(self, Entry::Alert(_))
    }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Entry::Group(n) => write!(f, "group:{n}"),
            Entry::Failed(n) => write!(f, "failed:{n}"),
            Entry::Gone => write!(f, "gone"),
            Entry::Alert(n) => write!(f, "alert:{n}"),
        }
    }
}

/// A tab of the open group: one of its questions, or its Submit tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    /// The question with this id.
    Question(i64),
    /// "Review your answers".
    Submit,
}

impl fmt::Display for Tab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tab::Question(n) => write!(f, "q:{n}"),
            Tab::Submit => write!(f, "submit"),
        }
    }
}

/// What the popup shows (Python `InboxApp.mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mode {
    /// Nothing open ("No open questions", or arrivals waiting for Tab).
    Empty,
    /// The group list.
    List,
    /// Inside a group: its questions as tabs.
    Group,
    /// An undelivered answer's view.
    Failed,
    /// An alert's view.
    Alert,
}

impl Mode {
    /// The golden's name (Python's string).
    pub fn name(self) -> &'static str {
        match self {
            Mode::Empty => "empty",
            Mode::List => "list",
            Mode::Group => "group",
            Mode::Failed => "failed",
            Mode::Alert => "alert",
        }
    }
}

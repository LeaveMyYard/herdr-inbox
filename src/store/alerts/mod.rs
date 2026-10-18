//! Alerts (Python `alerts.py`): "please check X" from an agent, with no answer
//! expected, and automatic alerts for agents stuck at an approval or question
//! prompt. The store side, statement for statement. What Python does to herdr
//! afterwards (a notification, the pane's markers) comes back as [`Effect`]s
//! for the caller to carry out.

mod agent;
mod blocked;

use rusqlite::Row;

pub use agent::{Asker, RaiseOutcome, WithdrawOutcome};
pub use blocked::{BlockedScan, SeenAgent};

/// Severities an agent may give (Python `SEVERITIES`); anything else is `warning`.
pub const SEVERITIES: [&str; 3] = ["info", "warning", "urgent"];
/// The longest title and message, in characters (Python `MAX_TITLE`, `MAX_MESSAGE`).
pub const MAX_TITLE: usize = 120;
/// See [`MAX_TITLE`].
pub const MAX_MESSAGE: usize = 2000;

const COLUMNS: &str = "id, created_at, kind, severity, title, message, pane_id, tab_id, tab_label, session_id, \
                       asked_by, herdr_server, status, acked_at, closed_at, reported_at";

/// One row of `alerts` (Python `alerts.Alert`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Alert {
    pub id: i64,
    pub created_at: String,
    /// `agent` (raised by an agent) or `blocked` (the scan's).
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub message: Option<String>,
    pub pane_id: Option<String>,
    pub tab_id: Option<String>,
    pub tab_label: Option<String>,
    pub session_id: Option<String>,
    pub asked_by: Option<String>,
    pub herdr_server: Option<String>,
    /// `open`, `acked`, `withdrawn`, `cleared`.
    pub status: String,
    pub acked_at: Option<String>,
    pub closed_at: Option<String>,
    pub reported_at: Option<String>,
}

impl Alert {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Alert> {
        Ok(Alert {
            id: r.get(0)?,
            created_at: r.get(1)?,
            kind: r.get(2)?,
            severity: r.get(3)?,
            title: r.get(4)?,
            message: r.get(5)?,
            pane_id: r.get(6)?,
            tab_id: r.get(7)?,
            tab_label: r.get(8)?,
            session_id: r.get(9)?,
            asked_by: r.get(10)?,
            herdr_server: r.get(11)?,
            status: r.get(12)?,
            acked_at: r.get(13)?,
            closed_at: r.get(14)?,
            reported_at: r.get(15)?,
        })
    }

    /// `A<id>`.
    pub fn reference(&self) -> String {
        format!("A{}", self.id)
    }
}

/// What Python does to herdr after a change, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `herdr.notify(title, body)`.
    Notify { title: String, body: String },
    /// `service.update_markers(pane)`: the pane's count token and tab mark.
    Markers(Option<String>),
}

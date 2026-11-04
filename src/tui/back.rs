//! What the popup's background threads report to its own thread (Python's
//! `call_from_thread`): the loop turns each into a model event or a toast.

use crate::tui::toasts::Toast;

/// A background thread's report.
#[derive(Debug)]
pub enum Back {
    /// A send was recorded and handed off (Python `_after_submit`): the
    /// questions it answered (or retried) and how many it was asked to send.
    /// The loop reads the database and gives the model
    /// [`Event::Sent`](inbox_core::popup::Event::Sent).
    Sent {
        /// The questions.
        done: Vec<i64>,
        /// How many were asked for.
        wanted: usize,
    },
    /// Going to an agent finished (Python `_focused`).
    Focused {
        /// The pane.
        pane: String,
        /// herdr's error, if it failed.
        error: Option<String>,
    },
    /// A notice only.
    Toast(Toast),
}

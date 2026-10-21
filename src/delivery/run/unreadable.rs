//! The once-per-pane notice for an input box that stays unreadable (Python
//! `service._note_unreadable`): a stall from a Claude Code UI change says
//! why instead of only the generic "not delivered" after 5 minutes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use inbox_core::deliver::Composer;

/// An input box unreadable this long gets one notification (Python
/// `UNREADABLE_NOTIFY_AFTER`).
pub const UNREADABLE_NOTIFY_AFTER: Duration = Duration::from_secs(60);

/// Per process: since when each pane's box is unreadable, and the panes told.
#[derive(Debug)]
pub struct Unreadable {
    after: Duration,
    since: BTreeMap<String, Instant>,
    told: BTreeSet<String>,
}

impl Default for Unreadable {
    fn default() -> Self {
        Unreadable::after(UNREADABLE_NOTIFY_AFTER)
    }
}

impl Unreadable {
    /// Tells once a box stayed unreadable for `after` (tests: 0).
    pub fn after(after: Duration) -> Self {
        Unreadable {
            after,
            since: BTreeMap::new(),
            told: BTreeSet::new(),
        }
    }

    /// The notification to show now for `pane` whose box read as `state`, if
    /// any (`where_`: its tab label or the pane; `refs`: `Q1, Q2`).
    pub fn note(
        &mut self,
        pane: &str,
        state: Composer,
        where_: &str,
        refs: &str,
        now: Instant,
    ) -> Option<(String, String)> {
        if state != Composer::Unknown {
            self.since.remove(pane);
            self.told.remove(pane);
            return None;
        }
        let first = *self.since.entry(pane.to_string()).or_insert(now);
        if self.told.contains(pane) || now.duration_since(first) < self.after {
            return None;
        }
        self.told.insert(pane.to_string());
        Some((
            format!("{where_}: cannot read the agent's input box"),
            format!(
                "{refs} wait; no answer is typed until it can be read (Claude Code UI changed? check herdr-inbox)"
            ),
        ))
    }
}

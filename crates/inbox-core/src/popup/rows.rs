//! The group list's rows: which entries the database holds and in what
//! order, the gone-agents summary row, and why a row or a tab closed.

use super::entry::{Entry, Tab};
use super::state::Popup;

impl Popup {
    /// The entries the database holds now (Python `_alert_entries() +
    /// _collapse_gone(entry_order(...)) + _remote_entries()`): alerts first
    /// (a remote machine's after the local ones), then groups (the focused
    /// pane's first, then oldest first), then ⚠ answers, then the remote
    /// machines' rows.
    pub(super) fn current_entries(&mut self, focus: Option<&str>) -> Vec<Entry> {
        let mut out: Vec<Entry> = self
            .data
            .open_alerts
            .iter()
            .map(|&a| Entry::Alert(a))
            .collect();
        out.extend(self.remote_alert_entries());
        let ordered = self.entry_order(focus);
        out.extend(self.collapse_gone(ordered));
        out.extend(self.remote_entries());
        out
    }

    /// Python `entry_order`: one row per group of open questions, sorted by
    /// (not the focused pane's, first question id); then the failed ones.
    fn entry_order(&self, focus: Option<&str>) -> Vec<Entry> {
        let mut firsts: Vec<(i64, i64, Option<&str>)> = Vec::new(); // (group, first id, its pane)
        let mut open: Vec<i64> = self.data.open.clone();
        open.sort_unstable();
        for id in open {
            let Some(q) = self.data.question(id) else {
                continue;
            };
            if !firsts.iter().any(|(g, _, _)| *g == q.group) {
                firsts.push((q.group, q.id, q.pane_id.as_deref()));
            }
        }
        firsts.sort_by_key(|(_, first, pane)| (*pane != focus, *first));
        let groups = firsts.into_iter().map(|(g, _, _)| Entry::Group(g));
        groups
            .chain(self.data.failed.iter().map(|&id| Entry::Failed(id)))
            .collect()
    }

    /// Python `_collapse_gone`: two or more gone-agent answers when the popup
    /// opens become ONE summary row (at the end); the set is fixed then, so
    /// later ones keep rows of their own.
    fn collapse_gone(&mut self, entries: Vec<Entry>) -> Vec<Entry> {
        let gone: Vec<i64> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::Failed(id) if self.data.question(*id).is_some_and(|q| q.agent_gone()) => {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        let batch = self.gone_batch.get_or_insert_with(|| {
            if gone.len() >= 2 {
                gone.clone()
            } else {
                Vec::new()
            }
        });
        let batch = batch.clone();
        let mut out: Vec<Entry> = entries
            .into_iter()
            .filter(|e| !matches!(e, Entry::Failed(id) if batch.contains(id)))
            .collect();
        if gone.iter().any(|id| batch.contains(id)) {
            out.push(Entry::Gone);
        }
        out
    }

    /// The summary row's answers still gone and shown (Python `gone_live`).
    pub(super) fn gone_batch_live(&self) -> Vec<i64> {
        let batch = self.gone_batch.as_deref().unwrap_or_default();
        batch
            .iter()
            .copied()
            .filter(|id| {
                self.data.failed.contains(id)
                    && self.data.question(*id).is_some_and(|q| q.agent_gone())
            })
            .collect()
    }

    /// Rows that are not placeholders.
    pub(super) fn live_entries(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| !self.closed.contains(e))
            .cloned()
            .collect()
    }

    /// The operator moved: placeholders drop out, except the one under the
    /// cursor (Python `purge_closed`).
    pub(super) fn purge_closed(&mut self, keep: Option<&Entry>) {
        let closed = &self.closed;
        self.entries
            .retain(|e| !closed.contains(e) || Some(e) == keep);
        self.closed.retain(|e| Some(e) == keep);
    }

    /// Why a list row ended (Python `_entry_reason`).
    pub(super) fn entry_reason(&self, entry: &Entry) -> String {
        if let Some(reason) = Popup::remote_reason(entry) {
            return reason.to_string();
        }
        match entry {
            Entry::Alert(id) => match self.data.alert(*id).map(|a| a.status.as_str()) {
                Some("acked") => "acknowledged",
                Some("withdrawn") => "withdrawn by the agent",
                Some("cleared") => "no longer waiting",
                _ => "closed",
            }
            .to_string(),
            Entry::Failed(id) => self.failed_reason(*id),
            Entry::Gone => "hidden or delivered".to_string(),
            Entry::Group(g) => {
                let mut reasons: Vec<String> = self
                    .data
                    .group(*g)
                    .iter()
                    .map(|q| self.tab_reason(&Tab::Question(q.id)))
                    .collect();
                reasons.sort();
                reasons.dedup();
                match &reasons[..] {
                    [one] => one.clone(),
                    _ => "closed".to_string(),
                }
            }
            _ => "closed".to_string(), // the remote rows' reasons are above
        }
    }

    /// Why a question tab closed (Python `_gone_reason("q", id)`).
    pub(super) fn tab_reason(&self, tab: &Tab) -> String {
        let Tab::Question(id) = tab else {
            return "closed".to_string();
        };
        match self.data.question(*id).map(|q| q.status.as_str()) {
            None => "removed".to_string(),
            Some("withdrawn") => "withdrawn by the agent".to_string(),
            Some(status) => status.to_string(),
        }
    }

    /// Why a ⚠ entry ended (Python `_gone_reason("failed", id)`).
    pub(super) fn failed_reason(&self, id: i64) -> String {
        match self.data.question(id) {
            Some(q) if q.delivered => "delivered".to_string(),
            _ => "no longer pending".to_string(),
        }
    }
}

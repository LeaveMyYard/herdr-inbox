//! Background events (Python `poll`, `_after_submit`, `_focused`). A poll
//! never changes what the next key does: arrivals are appended (armed
//! [`ROW_ARM`](super::ROW_ARM) later), ended rows and tabs become inert
//! placeholders, nothing joins an open group, and the view on screen is
//! replaced only when what it shows ended, with the [`GRACE`] window.

use std::collections::HashSet;

use super::GRACE;
use super::data::Snapshot;
use super::entry::{Entry, Mode, Tab};
use super::event::Severity;
use super::state::Popup;
use super::views::{GoneView, View};

impl Popup {
    pub(super) fn poll(&mut self, snapshot: Snapshot) {
        self.data = snapshot;
        if self.confirm.is_some() {
            return; // a confirmation is open: the rows wait for it
        }
        let current = self.current_entries(None);
        let live: HashSet<&Entry> = current.iter().collect();
        let ended: Vec<Entry> = self
            .entries
            .iter()
            .filter(|e| !live.contains(e) && !self.closed.contains(e))
            .cloned()
            .collect();
        for e in ended {
            let reason = self.entry_reason(&e);
            self.closed.insert(e, reason);
        }
        let new: Vec<Entry> = current
            .into_iter()
            .filter(|e| !self.entries.contains(e))
            .collect();
        for e in &new {
            self.arrived_at.insert(e.clone(), self.now);
        }
        self.entries.extend(new);
        let on_screen_ended = self
            .entry
            .as_ref()
            .and_then(|e| self.closed.get(e))
            .cloned();
        let placeholder = matches!(self.view, View::Gone(_));
        match (self.mode, self.entry.clone(), on_screen_ended) {
            (Mode::Failed, Some(Entry::Failed(id)), Some(reason)) if !placeholder => {
                self.replace_with_placeholder(format!("Q{id}"), reason)
            }
            (Mode::Alert, Some(Entry::Alert(id)), Some(reason)) if !placeholder => {
                self.replace_with_placeholder(format!("A{id}"), reason)
            }
            (Mode::Group, _, _) => self.poll_group(),
            _ => {}
        }
    }

    fn replace_with_placeholder(&mut self, r#ref: String, reason: String) {
        self.timers.grace_until = Some(self.now + GRACE);
        self.mount(View::Gone(GoneView { r#ref, reason }));
    }

    /// The open group under a poll (Python `_poll_group`): its closed tabs
    /// stay as placeholders; the view on screen is rebuilt only if its own
    /// question closed.
    fn poll_group(&mut self) {
        let open: HashSet<i64> = self.data.open.iter().copied().collect();
        let is_open = |t: &Tab| matches!(t, Tab::Question(id) if open.contains(id));
        let ended: Vec<Tab> = self
            .tabs
            .iter()
            .filter(|t| !is_open(t) && !self.gone.contains(t))
            .copied()
            .collect();
        let (current, gone) = (self.current, &self.gone);
        self.tabs
            .retain(|t| is_open(t) || Some(*t) == current || gone.contains(t));
        if let Some(cur) = self.current.filter(|c| ended.contains(c)) {
            let reason = self.tab_reason(&cur);
            self.notify(
                &format!("{} was closed ({reason})", tab_ref(cur)),
                Severity::Warning,
            );
            self.gone.insert(cur, reason);
            self.timers.grace_until = Some(self.now + GRACE);
            return self.show(Some(cur)); // the same tab, now the placeholder
        }
        let single = self.single;
        let qid = match &self.view {
            View::Question(v) => Some(v.qid),
            _ => None,
        };
        let last = qid.map(|q| self.last_open(q));
        let open_qids = self.open_qids();
        match &mut self.view {
            View::Question(v) => {
                (v.in_group, v.last) = (!single, last.unwrap_or(v.last));
            }
            View::Review(v) => {
                v.qids = open_qids;
                v.reload(&self.data);
            }
            _ => {}
        }
    }

    /// A send was recorded (Python `_after_submit`, detached).
    pub(super) fn sent(&mut self, done: &[i64], wanted: usize, snapshot: Snapshot) {
        if done.is_empty() {
            self.notify(
                "Nothing sent: those questions are no longer open",
                Severity::Warning,
            );
        } else {
            let refs: Vec<String> = done.iter().map(|q| format!("Q{q}")).collect();
            let text = format!(
                "Sending {} (in the background; see ⏳/⚠ here if it waits)",
                refs.join(", ")
            );
            self.notify(&text, Severity::Information);
        }
        if !done.is_empty() && done.len() < wanted {
            let text = format!(
                "{} answered question(s) had closed meanwhile",
                wanted - done.len()
            );
            self.notify(&text, Severity::Warning);
        }
        self.poll(snapshot);
        if matches!(self.mode, Mode::List | Mode::Empty)
            && self.live_entries().is_empty()
            && !done.is_empty()
        {
            self.exit(); // like Claude Code's prompt: gone once everything is answered
        }
    }

    /// A go-to-agent finished (Python `_focused`).
    pub(super) fn focused(&mut self, pane: &str, error: Option<&str>) {
        match error {
            Some(err) => self.notify(&format!("could not focus {pane}: {err}"), Severity::Warning),
            None => self.exit(), // the question stays open; discuss it in the agent's chat
        }
    }
}

fn tab_ref(tab: Tab) -> String {
    match tab {
        Tab::Question(id) => format!("Q{id}"),
        Tab::Submit => "Submit".to_string(),
    }
}

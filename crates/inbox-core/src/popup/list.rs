//! The group list's keys (Python `GroupListView`): ↑/↓ (no wrap-around, so
//! an arrival is never where ↓ wraps to), digits name question rows (alerts
//! are unnumbered), Enter opens, Esc closes the popup. A row that arrived
//! less than [`ROW_ARM`] ago takes no digit and no Enter; a placeholder
//! takes no Enter.

use super::ROW_ARM;
use super::entry::Entry;
use super::key::Key;
use super::state::{Msg, Popup};
use super::views::View;

impl Popup {
    /// The list view's cursor (the row under it now); [`Popup::list_cursor`]
    /// is the row the popup returns to from a group.
    pub(super) fn list_target(&self) -> Option<Entry> {
        match &self.view {
            View::List(v) => v.cursor.clone(),
            _ => None,
        }
    }

    fn set_list_cursor(&mut self, cursor: Option<Entry>) {
        if let View::List(v) = &mut self.view {
            v.cursor = cursor;
        }
    }

    /// The row arrived in the background less than [`ROW_ARM`] ago.
    pub fn fresh(&self, entry: &Entry) -> bool {
        self.arrived_at
            .get(entry)
            .is_some_and(|&at| self.now < at + ROW_ARM)
    }

    /// Take a key in the list; `false` if the list does not bind it.
    pub(super) fn list_key(&mut self, key: &Key, armed: bool) -> bool {
        match key {
            Key::Up | Key::Char('k') => self.list_move(-1),
            Key::Down | Key::Char('j') => self.list_move(1),
            Key::Escape => self.later(Msg::Quit),
            Key::Enter if armed => {
                let cursor = self.list_target();
                if let Some(c) = cursor.filter(|c| {
                    self.entries.contains(c) && !self.closed.contains(c) && !self.fresh(c)
                }) {
                    self.later(Msg::Open(c));
                }
            }
            _ => match key.digit() {
                Some(n) if armed => {
                    let numbered: Vec<Entry> = self
                        .entries
                        .iter()
                        .filter(|e| !e.is_alert())
                        .cloned()
                        .collect();
                    if let Some(e) = numbered.get(n - 1).filter(|e| !self.fresh(e)).cloned() {
                        self.set_list_cursor(Some(e));
                        self.list_moved();
                    }
                }
                _ => return false,
            },
        }
        true
    }

    fn list_move(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let cursor = self.list_target();
        let next = match cursor.and_then(|c| self.entries.iter().position(|e| *e == c)) {
            None if delta > 0 => self.entries[0].clone(),
            None => self.entries[self.entries.len() - 1].clone(),
            Some(i) => self.entries
                [(i as isize + delta).clamp(0, self.entries.len() as isize - 1) as usize]
                .clone(),
        };
        self.set_list_cursor(Some(next));
        self.list_moved();
    }

    /// A deliberate move: placeholders drop out now, the target stays.
    fn list_moved(&mut self) {
        let keep = self.list_target();
        self.purge_closed(keep.as_ref());
    }
}

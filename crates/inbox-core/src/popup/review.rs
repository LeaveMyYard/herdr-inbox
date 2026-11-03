//! The open group's Submit tab, "Review your answers" (Python `ReviewView`):
//! every open question with its draft, then Submit answers / Cancel. Its
//! cursor is a target; when its question goes it parks on nothing.

use super::data::{Millis, Snapshot};
use super::event::Severity;
use super::key::Key;
use super::state::Msg;
use super::views::Out;

/// Where the Submit tab's cursor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCursor {
    /// A question's row (Enter edits it, `x` drops its draft).
    Item(i64),
    /// "Submit answers".
    Submit,
    /// "Cancel".
    Cancel,
}

/// The Submit tab's state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReviewView {
    /// The group's open question tabs when it was shown (or last polled).
    pub qids: Vec<i64>,
    /// Those still open, in order.
    pub items: Vec<i64>,
    /// Those of them with a draft, as read with them: Submit answers sends
    /// these (Python reads the drafts when the tab appears or reloads, not
    /// at the key).
    pub drafted: Vec<i64>,
    /// The cursor; `None` is the inert rest position (Enter does nothing).
    pub cursor: Option<ReviewCursor>,
    /// Enter and 1/2 count from then on.
    pub armed_at: Millis,
}

impl ReviewView {
    pub(super) fn new(qids: Vec<i64>, data: &Snapshot, armed_at: Millis) -> ReviewView {
        let mut v = ReviewView {
            qids,
            items: Vec::new(),
            drafted: Vec::new(),
            cursor: Some(ReviewCursor::Submit),
            armed_at,
        };
        v.reload(data);
        v
    }

    /// Re-read its questions; a cursor whose question went parks (Python `reload`).
    pub(super) fn reload(&mut self, data: &Snapshot) {
        self.items = self
            .qids
            .iter()
            .copied()
            .filter(|&id| data.question(id).is_some_and(|q| q.status == "open"))
            .collect();
        self.drafted = self
            .items
            .iter()
            .copied()
            .filter(|&id| data.question(id).is_some_and(|q| q.draft.is_some()))
            .collect();
        if self.cursor.is_some_and(|c| !self.targets().contains(&c)) {
            self.cursor = None;
        }
    }

    /// Its rows' targets, in order.
    pub fn targets(&self) -> Vec<ReviewCursor> {
        let mut t: Vec<ReviewCursor> = self
            .items
            .iter()
            .map(|&id| ReviewCursor::Item(id))
            .collect();
        t.extend([ReviewCursor::Submit, ReviewCursor::Cancel]);
        t
    }

    /// Take a key; `false` if the view does not bind it. `x` (drop the
    /// row's draft) is the caller's: [`ReviewView::drop_target`].
    pub(super) fn key(&mut self, key: &Key, armed: bool, out: &mut Out) -> bool {
        match key {
            Key::Up | Key::Char('k') => self.step(-1),
            Key::Down | Key::Char('j') => self.step(1),
            Key::Escape => out.later(Msg::Back),
            Key::Enter if armed => self.select(out),
            Key::Char(c @ ('1' | '2')) if armed => {
                self.cursor = Some(if *c == '1' {
                    ReviewCursor::Submit
                } else {
                    ReviewCursor::Cancel
                });
                self.select(out);
            }
            _ => return false,
        }
        true
    }

    /// The question whose draft `x` drops (the cursor's row).
    pub(super) fn drop_target(&self) -> Option<i64> {
        match self.cursor {
            Some(ReviewCursor::Item(id)) => Some(id),
            _ => None,
        }
    }

    fn step(&mut self, delta: isize) {
        let targets = self.targets();
        let n = targets.len() as isize;
        self.cursor = Some(
            match self
                .cursor
                .and_then(|c| targets.iter().position(|t| *t == c))
            {
                None if delta > 0 => targets[0],
                None => targets[targets.len() - 1],
                Some(i) => targets[(i as isize + delta).rem_euclid(n) as usize],
            },
        );
    }

    fn select(&self, out: &mut Out) {
        match self.cursor {
            None => {}
            Some(ReviewCursor::Item(id)) => out.later(Msg::Edit(id)),
            Some(ReviewCursor::Submit) => {
                let drafted = self.drafted.clone();
                if drafted.is_empty() {
                    out.notify("Nothing answered yet", Severity::Warning);
                } else {
                    out.later(Msg::Submit(drafted));
                }
            }
            Some(ReviewCursor::Cancel) => out.later(Msg::Back),
        }
    }
}

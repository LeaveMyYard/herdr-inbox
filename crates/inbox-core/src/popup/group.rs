//! Moving between views on the operator's actions: opening a list row,
//! the open group's tabs, back to the list, the empty view. A view mounted
//! here is armed [`VIEW_ARM`] later.

use super::VIEW_ARM;
use super::entry::{Entry, Mode, Tab};
use super::event::Severity;
use super::question::QuestionView;
use super::review::ReviewView;
use super::state::{Msg, Popup};
use super::views::{AlertView, FailedView, GoneView, ListView, View};

impl Popup {
    /// Mount a view (Python `_mount`): the view change is stamped.
    pub(super) fn mount(&mut self, view: View) {
        self.timers.view_at = Some(self.now);
        self.view = view;
        self.focus = super::focus::Focus::View;
    }

    pub(super) fn armed_at(&self) -> u64 {
        self.now + VIEW_ARM
    }

    /// Open a list row (Python `open_group`): a group's questions as tabs,
    /// an alert's or a ⚠ answer's view.
    pub(super) fn open_entry(&mut self, entry: &Entry, start: Option<i64>) {
        self.list_cursor = Some(entry.clone());
        match entry {
            Entry::Alert(id) => {
                if self.data.alert(*id).is_none_or(|a| a.status != "open") {
                    return self.show_list(Some(entry.clone()));
                }
                (self.mode, self.entry) = (Mode::Alert, Some(entry.clone()));
                (self.tabs, self.current) = (Vec::new(), None);
                let armed_at = self.armed_at();
                self.mount(View::Alert(AlertView {
                    alert: *id,
                    armed_at,
                }));
            }
            Entry::Gone => self.notify(
                "Those agents are gone: d hides all these answers (check_answers still has them)",
                Severity::Information,
            ),
            Entry::Failed(id) => {
                (self.mode, self.entry) = (Mode::Failed, Some(entry.clone()));
                let armed_at = self.armed_at();
                self.mount(View::Failed(FailedView {
                    qid: *id,
                    cursor: 0,
                    armed_at,
                }));
            }
            Entry::Group(g) => {
                let qs: Vec<(i64, bool)> = self
                    .data
                    .open_group(*g)
                    .iter()
                    .map(|q| (q.id, q.draft.is_some()))
                    .collect();
                if qs.is_empty() {
                    return self.show_list(Some(entry.clone()));
                }
                (self.mode, self.entry) = (Mode::Group, Some(entry.clone()));
                self.tabs = qs.iter().map(|&(id, _)| Tab::Question(id)).collect();
                self.gone = Default::default();
                self.single = qs.len() == 1; // decided here, on the operator's move
                let blank = qs
                    .iter()
                    .find(|(_, drafted)| !drafted)
                    .map_or(qs[0].0, |&(id, _)| id);
                let start = start
                    .filter(|id| qs.iter().any(|(q, _)| q == id))
                    .unwrap_or(blank);
                self.show(Some(Tab::Question(start)));
            }
        }
    }

    /// The open group's questions still open (no placeholders).
    pub(super) fn open_qids(&self) -> Vec<i64> {
        self.tabs
            .iter()
            .filter(|t| !self.gone.contains(t))
            .filter_map(|t| match t {
                Tab::Question(id) => Some(*id),
                Tab::Submit => None,
            })
            .collect()
    }

    /// The tabs, with the Submit tab unless a lone question or nothing is open.
    pub fn all_tabs(&self) -> Vec<Tab> {
        let mut tabs = self.tabs.clone();
        if !self.single && !self.open_qids().is_empty() {
            tabs.push(Tab::Submit);
        }
        tabs
    }

    pub(super) fn last_open(&self, qid: i64) -> bool {
        self.single || self.open_qids().last() == Some(&qid)
    }

    /// Show a tab of the open group (Python `show`).
    pub(super) fn show(&mut self, tab: Option<Tab>) {
        self.current = tab;
        let Some(tab) = tab else {
            return self.back_to_list(false);
        };
        let armed_at = self.armed_at();
        let view = match tab {
            Tab::Submit => View::Review(ReviewView::new(self.open_qids(), &self.data, armed_at)),
            Tab::Question(id) => match self.gone.get(&tab) {
                Some(reason) => View::Gone(GoneView {
                    r#ref: format!("Q{id}"),
                    reason: reason.clone(),
                }),
                None => match self.data.question(id) {
                    Some(q) => View::Question(QuestionView::new(
                        q,
                        !self.single,
                        self.last_open(id),
                        armed_at,
                    )),
                    None => View::Gone(GoneView {
                        r#ref: format!("Q{id}"),
                        reason: "removed".to_string(),
                    }),
                },
            },
        };
        self.mount(view);
    }

    /// Move between the open group's tabs, by `delta` or to a tab;
    /// placeholders drop out on this move (Python `navigate`).
    pub(super) fn navigate(&mut self, delta: isize, to: Option<Tab>) {
        let tabs = self.all_tabs();
        let target = match to {
            Some(t) => Some(t),
            None if tabs.is_empty() => None,
            None => {
                let i = self
                    .current
                    .and_then(|c| tabs.iter().position(|t| *t == c))
                    .unwrap_or(0) as isize;
                Some(tabs[(i + delta).rem_euclid(tabs.len() as isize) as usize])
            }
        };
        let gone = &self.gone;
        self.tabs
            .retain(|t| !gone.contains(t) || Some(*t) == target);
        self.gone.retain(|t| Some(*t) == target);
        let tabs = self.all_tabs();
        let target = target
            .filter(|t| tabs.contains(t))
            .or_else(|| tabs.first().copied());
        self.show(target);
    }

    /// The group list (Python `show_list`).
    pub(super) fn show_list(&mut self, cursor: Option<Entry>) {
        (self.mode, self.entry, self.direct) = (Mode::List, None, false);
        (self.tabs, self.current, self.gone) = (Vec::new(), None, Default::default());
        if self.entries.is_empty() {
            return self.show_empty();
        }
        let cursor = cursor
            .or_else(|| self.list_cursor.clone())
            .filter(|c| self.entries.contains(c));
        let armed_at = self.armed_at();
        self.mount(View::List(ListView { cursor, armed_at }));
    }

    /// Nothing open (Python `show_empty`).
    pub(super) fn show_empty(&mut self) {
        self.mode = Mode::Empty;
        self.mount(View::None);
    }

    /// After a send, hide or cancel: the list, cursor still on the entry
    /// just handled; close if nothing is left. Open alerts keep the popup
    /// open only after an alert action (Python `back_to_list`).
    pub(super) fn back_to_list(&mut self, from_alert: bool) {
        self.direct = false;
        let left = self
            .live_entries()
            .into_iter()
            .any(|e| from_alert || !e.is_alert());
        if !left {
            return self.later(Msg::Quit);
        }
        self.show_list(self.list_cursor.clone());
    }

    /// Esc (Python `action_back`): back to the list, or close the popup
    /// (from the list, or when it opened straight into the only group).
    pub(super) fn back(&mut self) {
        if matches!(self.mode, Mode::Group | Mode::Failed | Mode::Alert) && !self.direct {
            return self.show_list(self.list_cursor.clone());
        }
        self.quit();
    }
}

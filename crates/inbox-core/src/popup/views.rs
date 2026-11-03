//! The views the popup's body shows, and the keys the small ones take (the
//! question and Submit views have files of their own).

use std::collections::VecDeque;

use super::data::Millis;
use super::entry::Entry;
use super::event::{Effect, Severity};
use super::key::Key;
use super::question::QuestionView;
use super::review::ReviewView;
use super::state::Msg;

/// The popup's body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// Nothing (the empty view).
    None,
    /// The group list.
    List(ListView),
    /// One question.
    Question(QuestionView),
    /// The open group's Submit tab.
    Review(ReviewView),
    /// An inert placeholder: what was on screen closed.
    Gone(GoneView),
    /// An undelivered answer: Retry, or stop showing it.
    Failed(FailedView),
    /// An alert: `d` acknowledges, `g` goes there; no Enter, no digits.
    Alert(AlertView),
}

/// The group list's own state (its rows are [`Popup::entries`](super::Popup::entries)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListView {
    /// The row under the cursor; `None` is the inert rest position.
    pub cursor: Option<Entry>,
    /// Enter and digits count from then on.
    pub armed_at: Millis,
}

/// "Q3 was closed (withdrawn by the agent)": Enter and digits do nothing;
/// only Esc or a deliberate tab move leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoneView {
    /// `Q3`, `A2`.
    pub r#ref: String,
    /// Why it closed.
    pub reason: String,
}

/// An undelivered answer's view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedView {
    /// Its question.
    pub qid: i64,
    /// 0: Retry delivery (Send now); 1: Stop showing it here.
    pub cursor: usize,
    /// Enter and digits count from then on.
    pub armed_at: Millis,
}

/// An alert's view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertView {
    /// The alert.
    pub alert: i64,
    /// `d` counts from then on.
    pub armed_at: Millis,
}

/// Where a view's key handler leaves effects and messages.
pub(super) struct Out<'a> {
    pub fx: &'a mut Vec<Effect>,
    pub queue: &'a mut VecDeque<Msg>,
}

impl Out<'_> {
    pub fn notify(&mut self, text: &str, severity: Severity) {
        self.fx.push(Effect::Notify {
            text: text.to_string(),
            severity,
        });
    }

    pub fn later(&mut self, msg: Msg) {
        self.queue.push_back(msg);
    }
}

impl GoneView {
    /// Esc goes back; nothing else is bound here.
    pub(super) fn key(&mut self, key: &Key, out: &mut Out) -> bool {
        if *key == Key::Escape {
            out.later(Msg::Back);
            return true;
        }
        false
    }
}

impl FailedView {
    /// ↑/↓, Enter, 1/2 (armed), Esc.
    pub(super) fn key(&mut self, key: &Key, armed: bool, out: &mut Out) -> bool {
        match key {
            Key::Up | Key::Down => {
                self.cursor = (self.cursor + 1) % 2;
                true
            }
            Key::Escape => {
                out.later(Msg::Back);
                true
            }
            Key::Enter if armed => {
                self.select(out);
                true
            }
            Key::Char(c @ ('1' | '2')) if armed => {
                self.cursor = usize::from(*c == '2');
                self.select(out);
                true
            }
            _ => false,
        }
    }

    fn select(&self, out: &mut Out) {
        out.later(if self.cursor == 0 {
            Msg::Retry(self.qid)
        } else {
            Msg::Hide(self.qid)
        });
    }
}

impl AlertView {
    /// `d` (armed) acknowledges; Esc goes back.
    pub(super) fn key(&mut self, key: &Key, armed: bool, out: &mut Out) -> bool {
        match key {
            Key::Char('d') if armed => {
                out.later(Msg::Ack(self.alert));
                true
            }
            Key::Escape => {
                out.later(Msg::Back);
                true
            }
            _ => false,
        }
    }
}

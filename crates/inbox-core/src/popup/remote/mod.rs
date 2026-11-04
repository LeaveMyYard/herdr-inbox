//! A remote machine's rows (Python `tui`'s `remote`, `routbox`, `rproblem`
//! and `ralert` entries, P6-D2): opening them, answering a remote question,
//! and what a poll does to the one on screen. The popup reads the local
//! mirror only ([`Remote`]); every write is an effect the IO layer turns into
//! a local database write (the delivery worker talks to the machine).
//!
//! One remote question at a time: [`Mode::Remote`] has no tabs and no Submit
//! tab, `d` and `g` do nothing in it, and Enter queues the answer under the
//! identity the question was shown with ([`RemoteRef`]).

mod data;
mod rows;

pub use data::{
    Machine, OutboxRow, Problem, Remote, RemoteAlert, RemoteGroup, RemoteQuestion, RemoteRef,
};

use super::data::{Detail, Question};
use super::entry::{Entry, Mode};
use super::event::{Effect, Severity};
use super::question::QuestionView;
use super::state::{Popup, Shown};
use super::views::{AlertView, View};

impl Popup {
    /// Open a remote row (Python `open_group`'s remote branches).
    pub(in crate::popup) fn open_remote(&mut self, entry: &Entry) {
        match entry {
            Entry::RemoteAlert(key) => {
                let Some(a) = self.data.remote.alert(key).cloned() else {
                    return self.show_list(Some(entry.clone()));
                };
                (self.mode, self.entry) = (Mode::Alert, Some(entry.clone()));
                (self.tabs, self.current) = (Vec::new(), None);
                let label = Some(self.data.remote.name(&a.machine_id));
                let armed_at = self.armed_at();
                self.mount(View::Alert(AlertView {
                    alert: a.alert.id,
                    label,
                    armed_at,
                }));
                self.remote_alert = Some(a);
            }
            Entry::Outbox(_) | Entry::Problem(_) => self.notify(
                "This is on a remote machine; d clears it here",
                Severity::Information,
            ),
            Entry::Remote(key) => {
                let Some(group) = self.data.remote.group(key) else {
                    return self.show_list(Some(entry.clone()));
                };
                let question = group.questions[0].clone();
                let label = self
                    .data
                    .remote
                    .machine(&group.machine_id)
                    .and_then(|m| m.label.clone());
                (self.mode, self.entry) = (Mode::Remote, Some(entry.clone()));
                (self.tabs, self.current) = (Vec::new(), None);
                let armed_at = self.armed_at();
                let view = QuestionView::new(&question.question, false, true, armed_at);
                self.remote_open = Some(Shown { label, question });
                self.mount(View::Question(view));
            }
            _ => {}
        }
    }

    /// The remote question's answer (Python `_answer_remote`): queued under
    /// the identity shown, unless the mirror holds another question there
    /// now; back to the list, the row an inert placeholder if it was the
    /// group's last question.
    pub(in crate::popup) fn answer_remote(&mut self, qid: i64, detail: Option<Detail>) {
        let Some(shown) = self.remote_open.clone() else {
            return;
        };
        let Some(detail) = detail.filter(|_| qid == shown.question.question.id) else {
            return;
        };
        let question = shown.question.r#ref();
        if let Some(why) = self.data.remote.refusal(&question) {
            return self.notify(&format!("not queued: {why}"), Severity::Warning);
        }
        let label = shown
            .label
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| question.machine_id.clone());
        let text = format!("{label}:Q{qid} answered · sending to {label}");
        self.data.remote.mark_busy(&question);
        self.fx.push(Effect::RemoteAnswer { question, detail });
        self.notify(&text, Severity::Information);
        if let Some(entry) = self.entry.clone() {
            let gone =
                matches!(&entry, Entry::Remote(key) if self.data.remote.group(key).is_none());
            if gone {
                self.closed
                    .insert(entry, format!("answered · sending to {label}"));
            }
        }
        self.back_to_list(false);
    }

    /// A poll in [`Mode::Remote`]: the question on screen closed over there
    /// (answered, withdrawn, dismissed): an inert placeholder, as a closed
    /// local tab.
    pub(in crate::popup) fn poll_remote(&mut self) {
        let (Some(Entry::Remote(key)), Some(shown)) = (&self.entry, &self.remote_open) else {
            return;
        };
        let id = shown.question.question.id;
        let open = self
            .data
            .remote
            .group(key)
            .is_some_and(|g| g.questions.iter().any(|q| q.question.id == id));
        if !open {
            // Python formats `m.get('label')`: an unlabelled machine reads "None"
            let label = shown.label.clone().unwrap_or_else(|| "None".to_string());
            self.replace_with_placeholder(
                format!("{label}:Q{id}"),
                "closed on the remote machine".to_string(),
            );
        }
    }

    /// The question a question view shows: in [`Mode::Remote`] the remote
    /// one (its id is the remote id, which a local question may share).
    pub fn view_question(&self, qid: i64) -> Option<&Question> {
        match (self.mode, &self.remote_open) {
            (Mode::Remote, Some(s)) if s.question.question.id == qid => Some(&s.question.question),
            (Mode::Remote, _) => None,
            _ => self.data.question(qid),
        }
    }

    /// The remote question shown in [`Mode::Remote`], and its machine's
    /// label as it was when it opened.
    pub fn remote_question(&self) -> Option<(&RemoteQuestion, Option<&str>)> {
        match (self.mode, &self.remote_open) {
            (Mode::Remote, Some(s)) => Some((&s.question, s.label.as_deref())),
            _ => None,
        }
    }

    /// The remote alert shown in [`Mode::Alert`], as it was when it opened.
    pub fn remote_alert(&self) -> Option<&RemoteAlert> {
        match (&self.entry, &self.remote_alert) {
            (Some(Entry::RemoteAlert(_)), Some(a)) if self.mode == Mode::Alert => Some(a),
            _ => None,
        }
    }
}

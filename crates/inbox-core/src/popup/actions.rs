//! What the operator's actions do (Python `InboxApp.on_*` and `action_*`):
//! answers, drafts, sends, hides, acknowledgements, dismissals and going to
//! an agent. Each emits its effect and keeps the model's copy of the
//! database as the write leaves it (the next snapshot confirms it).

use super::GO_PAUSE;
use super::data::Detail;
use super::entry::{Entry, Mode, Tab};
use super::event::{Effect, Send, Severity};
use super::state::{Confirm, ConfirmTarget, Msg, Popup};

impl Popup {
    pub(super) fn message(&mut self, msg: Msg) {
        match msg {
            Msg::Back => self.back(),
            Msg::Quit => self.quit(),
            Msg::Open(entry) => self.open_entry(&entry, None),
            Msg::Answered(qid, detail) => self.answered(qid, detail),
            Msg::Chat(qid) => {
                let pane = self.data.question(qid).map(|q| q.pane_id.clone());
                self.focus_pane(pane, false);
            }
            Msg::Edit(qid) => self.navigate(0, Some(Tab::Question(qid))),
            Msg::Submit(qids) => {
                let open = self.open_qids();
                let whole = open.iter().all(|q| qids.contains(q));
                let entry = self.entry.clone();
                self.send(Send::Submit { qids });
                self.after_send(entry, whole);
            }
            Msg::Retry(qid) => {
                let entry = self.entry.clone();
                self.send(Send::Retry { qid });
                self.after_send(entry, true);
            }
            Msg::Hide(qid) => {
                self.hide(qid);
                self.closed.insert(Entry::Failed(qid), "hidden".to_string());
                self.back_to_list(false);
            }
            Msg::Ack(_) => {
                let Some(entry) = self.entry.clone() else {
                    return;
                };
                if self.mode != Mode::Alert || self.closed.contains(&entry) {
                    return;
                }
                self.ack(&entry);
                self.back_to_list(true);
            }
        }
    }

    /// A question view's answer (Python `on_question_view_answered`).
    fn answered(&mut self, qid: i64, detail: Option<Detail>) {
        let tab = Tab::Question(qid);
        if self.mode != Mode::Group || self.gone.contains(&tab) || self.current != Some(tab) {
            return; // never act for a question that is not the one on screen
        }
        if self.single {
            // a lone question the operator opened: answer and send at once
            let entry = self.entry.clone();
            self.after_send(entry, true);
            self.send(Send::Answer { qid, detail });
            return;
        }
        if !self.write_draft(qid, detail.clone()) {
            self.notify(&format!("Q{qid} is no longer open"), Severity::Warning);
        }
        if detail.is_some() {
            self.navigate(1, None); // Claude Code moves to the next tab (the last one: Submit)
        }
    }

    /// Hand answers to delivery (Python `_send`).
    fn send(&mut self, send: Send) {
        let qids = match &send {
            Send::Answer { qid, .. } | Send::Retry { qid } => vec![*qid],
            Send::Submit { qids } => qids.clone(),
        };
        let refs: Vec<String> = qids.iter().map(|q| format!("Q{q}")).collect();
        self.notify(
            &format!("Sending {}…", refs.join(", ")),
            Severity::Information,
        );
        match &send {
            Send::Answer {
                qid,
                detail: Some(d),
            } => {
                if self.write_draft_quiet(*qid, Some(d.clone())) {
                    self.write_status(*qid, "answered");
                }
            }
            Send::Submit { qids } => {
                for &qid in qids {
                    if self
                        .data
                        .question(qid)
                        .is_some_and(|q| q.status == "open" && q.draft.is_some())
                    {
                        self.write_status(qid, "answered");
                    }
                }
            }
            Send::Answer { detail: None, .. } | Send::Retry { .. } => {}
        }
        self.fx.push(Effect::Send(send));
    }

    /// After the operator's send: back to the list now; a group sent in full
    /// stays as an inert row (Python `_sent`).
    fn after_send(&mut self, entry: Option<Entry>, whole: bool) {
        if let (Some(entry), true) = (entry, whole) {
            self.closed.insert(entry, "sending…".to_string());
        }
        self.back_to_list(false);
    }

    fn hide(&mut self, qid: i64) {
        self.fx.push(Effect::Hide { qid });
        self.data.failed.retain(|&q| q != qid);
    }

    fn ack(&mut self, entry: &Entry) {
        let Entry::Alert(id) = entry else { return };
        self.fx.push(Effect::Ack { alert: *id });
        if let Some(a) = self.data.alerts.iter_mut().find(|a| a.id == *id) {
            a.status = "acked".to_string();
        }
        self.data.open_alerts.retain(|a| a != id);
        self.closed
            .insert(entry.clone(), "acknowledged".to_string());
    }

    /// `d` (Python `action_dismiss`): on a list row, acknowledge an alert,
    /// hide gone-agent answers, or ask to dismiss a whole group; inside a
    /// group, ask to dismiss the question on screen.
    pub(super) fn dismiss(&mut self) {
        if self.mode == Mode::List {
            let Some(entry) = self.list_target().filter(|e| !self.closed.contains(e)) else {
                return;
            };
            match &entry {
                Entry::Alert(_) => self.ack(&entry),
                Entry::Gone => {
                    for qid in self.gone_batch_live() {
                        self.hide(qid);
                    }
                    self.closed.insert(entry, "hidden".to_string());
                }
                Entry::Failed(qid) if self.data.question(*qid).is_some_and(|q| q.agent_gone()) => {
                    self.hide(*qid);
                    self.closed.insert(entry, "hidden".to_string());
                }
                Entry::Failed(_) => {}
                Entry::Group(g) => {
                    let qs: Vec<i64> = self.data.open_group(*g).iter().map(|q| q.id).collect();
                    let what = match &qs[..] {
                        [] => return,
                        [one] => format!("Q{one}"),
                        [first, .., last] => format!("Q{first}–Q{last} ({} questions)", qs.len()),
                    };
                    self.confirm = Some(Confirm {
                        what,
                        target: ConfirmTarget::Group(entry, qs),
                    });
                }
            }
            return;
        }
        let Some(Tab::Question(qid)) = self
            .current
            .filter(|c| self.mode == Mode::Group && !self.gone.contains(c))
        else {
            return;
        };
        if self.data.question(qid).is_some() {
            self.confirm = Some(Confirm {
                what: format!("Q{qid}"),
                target: ConfirmTarget::Question(qid),
            });
        }
    }

    /// `g` (Python `action_goto`).
    pub(super) fn go_to(&mut self) {
        let pane = |p: &Popup, e: &Entry| match e {
            Entry::Alert(id) => p.data.alert(*id).map(|a| a.pane_id.clone()),
            Entry::Failed(id) => p.data.question(*id).map(|q| q.pane_id.clone()),
            Entry::Group(g) => p.data.open_group(*g).first().map(|q| q.pane_id.clone()),
            Entry::Gone => None,
        };
        match (self.mode, self.entry.clone()) {
            (Mode::List, _) => {
                if let Some(cursor) = self.list_target() {
                    let target = pane(self, &cursor);
                    self.focus_pane(target, cursor.is_alert());
                }
            }
            (Mode::Failed, Some(e)) | (Mode::Alert, Some(e)) => {
                let target = pane(self, &e);
                self.focus_pane(target, e.is_alert());
            }
            _ => {
                if let Some(Tab::Question(qid)) = self.current {
                    let target = self.data.question(qid).map(|q| q.pane_id.clone());
                    self.focus_pane(target, false);
                }
            }
        }
    }

    /// Go to a pane (Python `_focus_pane`): an alert's needs a deliberate `g`.
    fn focus_pane(&mut self, target: Option<Option<String>>, alert: bool) {
        let Some(pane) = target else { return };
        let Some(pane) = pane else {
            return self.notify("could not focus: no pane", Severity::Warning);
        };
        let since = |t: Option<u64>| t.is_none_or(|t| self.now >= t + GO_PAUSE);
        let graced = since(self.timers.prev_key_at) && since(self.timers.view_at);
        if alert && !graced {
            return self.notify(
                "Pause, then press g to go to that agent",
                Severity::Information,
            );
        }
        self.fx.push(Effect::GoTo { pane, graced });
    }

    /// Store a draft (the effect, and the model's copy if the question is
    /// open); `false` if it is not open (Python `set_draft`'s rowcount).
    pub(super) fn write_draft(&mut self, qid: i64, detail: Option<Detail>) -> bool {
        self.fx.push(Effect::Draft {
            qid,
            detail: detail.clone(),
        });
        self.write_draft_quiet(qid, detail)
    }

    fn write_draft_quiet(&mut self, qid: i64, detail: Option<Detail>) -> bool {
        match self.data.question_mut(qid).filter(|q| q.status == "open") {
            Some(q) => {
                q.draft = detail;
                true
            }
            None => false,
        }
    }

    /// Close an open question in the model's copy; `false` if it was not open.
    pub(super) fn write_status(&mut self, qid: i64, status: &str) -> bool {
        let Some(q) = self.data.question_mut(qid).filter(|q| q.status == "open") else {
            return false;
        };
        q.status = status.to_string();
        self.data.open.retain(|&id| id != qid);
        true
    }
}

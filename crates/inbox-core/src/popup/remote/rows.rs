//! A remote machine's list rows: which there are and in what order, why one
//! ended, and what `d` does on them (Python `_remote_entries`,
//! `_entry_reason`, `action_dismiss`, `_ack_entry`).

use super::RemoteQuestion;
use crate::popup::confirm::{Confirm, ConfirmTarget};
use crate::popup::entry::Entry;
use crate::popup::event::{Effect, Severity};
use crate::popup::state::Popup;

impl Popup {
    /// The remote alerts' rows (after the local alerts).
    pub(in crate::popup) fn remote_alert_entries(&self) -> Vec<Entry> {
        let alerts = &self.data.remote.alerts;
        alerts.iter().map(|a| Entry::RemoteAlert(a.key())).collect()
    }

    /// The remote rows at the end of the list: open groups, then outbox
    /// rows, then undeliverable answers.
    pub(in crate::popup) fn remote_entries(&self) -> Vec<Entry> {
        let r = &self.data.remote;
        let groups = r.groups.iter().map(|g| Entry::Remote(g.key.clone()));
        let outbox = r.outbox.iter().map(|o| Entry::Outbox(o.request_id.clone()));
        let problems = r.problems.iter().map(|p| Entry::Problem(p.key()));
        groups.chain(outbox).chain(problems).collect()
    }

    /// Why a remote row ended (an alert is gone from the open ones, so its
    /// reason is always "closed"), or `None` for a local row.
    pub(in crate::popup) fn remote_reason(entry: &Entry) -> Option<&'static str> {
        Some(match entry {
            Entry::RemoteAlert(_) => "closed",
            Entry::Remote(_) => "answered or closed",
            Entry::Outbox(_) => "sent",
            Entry::Problem(_) => "delivered or cleared",
            _ => return None,
        })
    }

    /// `d` on a remote list row (not a placeholder): acknowledge an alert,
    /// clear an outbox row that is clearable, clear an undeliverable answer,
    /// or ask to dismiss a remote group.
    pub(in crate::popup) fn dismiss_remote(&mut self, entry: Entry) {
        match &entry {
            Entry::RemoteAlert(_) => self.ack_remote(&entry),
            Entry::Outbox(id) => {
                let Some(row) = self.data.remote.outbox_row(id).filter(|r| r.clearable) else {
                    return; // still queued, or being sent right now: nothing to clear
                };
                let expire = row.state == "queued";
                self.fx.push(Effect::ClearOutbox {
                    request_id: id.clone(),
                    expire,
                });
                self.data.remote.outbox.retain(|r| r.request_id != *id);
                self.closed.insert(entry, "cleared".to_string());
            }
            Entry::Problem(key) => {
                let (machine_id, rid) = key.split_once('|').unwrap_or((key, ""));
                let Ok(remote_id) = rid.parse() else { return };
                self.fx.push(Effect::ClearProblem {
                    machine_id: machine_id.to_string(),
                    remote_id,
                });
                self.data.remote.problems.retain(|p| p.key() != *key);
                self.closed.insert(entry, "cleared".to_string());
            }
            Entry::Remote(key) => {
                let Some(group) = self.data.remote.group(key) else {
                    return;
                };
                let label = self.data.remote.name(&group.machine_id);
                let questions = group.questions.clone();
                self.confirm = Some(Confirm {
                    what: format!("{label}:Q{}", questions[0].question.id),
                    target: ConfirmTarget::Remote {
                        entry,
                        label,
                        questions,
                    },
                });
            }
            _ => {}
        }
    }

    /// Acknowledge a remote alert (if it is still open), and show it so.
    pub(in crate::popup) fn ack_remote(&mut self, entry: &Entry) {
        let Entry::RemoteAlert(key) = entry else {
            return;
        };
        if let Some(a) = self.data.remote.alert(key).cloned() {
            self.fx.push(Effect::RemoteAck {
                machine_id: a.machine_id,
                alert: a.alert.id,
                db_id: a.db_id,
                akey: a.akey,
            });
            self.data.remote.alerts.retain(|a| a.key() != *key);
        }
        self.closed
            .insert(entry.clone(), "acknowledged".to_string());
    }

    /// `y` on a remote group: a dismissal queued per question, under the
    /// identity each had when `d` was pressed. The model stops at the first
    /// one the mirror no longer holds (Python's `LookupError` there escapes
    /// the confirmation and ends the popup).
    pub(in crate::popup) fn confirmed_remote(
        &mut self,
        entry: Entry,
        label: String,
        questions: Vec<RemoteQuestion>,
    ) {
        for q in questions {
            let question = q.r#ref();
            if let Some(why) = self.data.remote.refusal(&question) {
                return self.notify(&format!("not queued: {why}"), Severity::Warning);
            }
            self.data.remote.mark_busy(&question);
            self.fx.push(Effect::RemoteDismiss { question });
        }
        self.closed.insert(entry, format!("dismissing on {label}"));
    }
}

//! Remote machines as the popup reads them: the local mirror only (Python
//! `remote.open_groups`, `open_alerts`, `outbox_rows`, `problem_rows`),
//! never the network. The IO layer reads it into [`Remote`] with the rest of
//! the [`Snapshot`](crate::popup::Snapshot); the wall-clock answers
//! (`expired`, `clearable`, the offline note) are read with it.

use crate::popup::data::{Alert, Question};

/// A mirrored question's identity: which question on which machine, as the
/// popup showed it. An action is queued under this identity only, never
/// under whatever the mirror holds under that number now (a reset remote
/// inbox can put another question there: Python `remote.enqueue`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RemoteRef {
    /// The machine (`remote_machines.machine_id`).
    pub machine_id: String,
    /// Its question id over there.
    pub remote_id: i64,
    /// Its inbox database's id (a recreated inbox has another).
    pub db_id: String,
    /// The question's own key.
    pub qkey: String,
}

/// A remote machine (`remote_machines`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// Its id.
    pub machine_id: String,
    /// Its label (herdr's name for it); `None` shows the id.
    pub label: Option<String>,
    /// "offline since 10:42 (3 h)" when the last pull failed (Python
    /// `remote.offline_note`), for the renderer.
    pub offline: Option<String>,
}

/// An open question on a remote machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteQuestion {
    /// Its machine.
    pub machine_id: String,
    /// Its inbox database's id.
    pub db_id: String,
    /// Its key.
    pub qkey: String,
    /// The question as the views show it (Python `remote.as_question`): its
    /// `id` is the remote id, `group` the remote group, `group_title` the
    /// remote title.
    pub question: Question,
}

impl RemoteQuestion {
    /// Its identity.
    pub fn r#ref(&self) -> RemoteRef {
        RemoteRef {
            machine_id: self.machine_id.clone(),
            remote_id: self.question.id,
            db_id: self.db_id.clone(),
            qkey: self.qkey.clone(),
        }
    }
}

/// A remote machine's open group: one list row (Python `remote.open_groups`:
/// questions with an answer or a dismissal already queued are left out).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteGroup {
    /// `<machine>|<db id>|<group>` (the row's [`Entry::Remote`](crate::popup::Entry::Remote)).
    pub key: String,
    /// Its machine.
    pub machine_id: String,
    /// Its open questions, in id order.
    pub questions: Vec<RemoteQuestion>,
}

/// A remote machine's open alert.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteAlert {
    /// Its machine.
    pub machine_id: String,
    /// Its inbox database's id.
    pub db_id: String,
    /// The alert's own key.
    pub akey: String,
    /// The alert as the views show it (Python `remote.as_alert`).
    pub alert: Alert,
}

impl RemoteAlert {
    /// Its row's key, `<machine>|<alert id>`.
    pub fn key(&self) -> String {
        format!("{}|{}", self.machine_id, self.alert.id)
    }
}

/// An action in the outbox: queued (⏳), or failed or refused (⚠) and not
/// yet cleared here (Python `remote.outbox_rows`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutboxRow {
    /// Its request id.
    pub request_id: String,
    /// Its machine.
    pub machine_id: String,
    /// `answer`, `dismiss`, `hide`, `ack_alert`.
    pub action: String,
    /// The question (or, for `ack_alert`, the alert) over there.
    pub remote_id: i64,
    /// `queued`, `rejected` or `failed`.
    pub state: String,
    /// Why it failed or was refused.
    pub result: Option<String>,
    /// Being sent right now (`sending_at` set).
    pub sending: bool,
    /// Queued past its expiry (until the sweep settles it).
    pub expired: bool,
    /// `d` clears it (Python `remote.clearable`: failed or refused, or past
    /// its expiry and not being sent right now).
    pub clearable: bool,
    /// Its expiry as the operator's clock shows it (`HH:MM`).
    pub expires: String,
}

/// A remote answer its machine applied but cannot deliver, not yet cleared
/// here (Python `remote.problem_rows`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Problem {
    /// Its machine.
    pub machine_id: String,
    /// The question over there.
    pub remote_id: i64,
    /// The machine's label, or its id.
    pub label: String,
    /// Why (`agent closed — answer kept for check_answers`, `not delivered: …`).
    pub why: String,
    /// The question's text.
    pub question: String,
    /// The answer.
    pub answer: Option<String>,
}

impl Problem {
    /// Its row's key, `<machine>|<question id>`.
    pub fn key(&self) -> String {
        format!("{}|{}", self.machine_id, self.remote_id)
    }
}

/// The remote machines' mirror at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Remote {
    /// Every mirrored machine.
    pub machines: Vec<Machine>,
    /// The open groups, one row each, in the mirror's order.
    pub groups: Vec<RemoteGroup>,
    /// Every mirrored question's identity, open or not: what an action
    /// queued now would be checked against.
    pub mirror: Vec<RemoteRef>,
    /// The open alerts, in order.
    pub alerts: Vec<RemoteAlert>,
    /// The outbox rows shown, oldest first.
    pub outbox: Vec<OutboxRow>,
    /// The answers that cannot be delivered, in order.
    pub problems: Vec<Problem>,
}

impl Remote {
    /// The machine `id`.
    pub fn machine(&self, id: &str) -> Option<&Machine> {
        self.machines.iter().find(|m| m.machine_id == id)
    }

    /// A machine's name in messages: its label, or its id (Python
    /// `m.get("label") or m["machine_id"]`).
    pub fn name(&self, id: &str) -> String {
        self.machine(id)
            .and_then(|m| m.label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| id.to_string())
    }

    /// The open group `key` (Python `remote.group_by_key`).
    pub fn group(&self, key: &str) -> Option<&RemoteGroup> {
        self.groups.iter().find(|g| g.key == key)
    }

    /// The open alert `key` (Python `remote.alert_by_key`).
    pub fn alert(&self, key: &str) -> Option<&RemoteAlert> {
        self.alerts.iter().find(|a| a.key() == key)
    }

    /// The outbox row `request_id`.
    pub fn outbox_row(&self, request_id: &str) -> Option<&OutboxRow> {
        self.outbox.iter().find(|r| r.request_id == request_id)
    }

    /// The undeliverable answer `key`.
    pub fn problem(&self, key: &str) -> Option<&Problem> {
        self.problems.iter().find(|p| p.key() == key)
    }

    /// Why an action on `r` would be refused now (Python `remote.enqueue`'s
    /// `LookupError`), or `None`: it goes under the identity shown.
    pub fn refusal(&self, r: &RemoteRef) -> Option<&'static str> {
        let row = self
            .mirror
            .iter()
            .find(|m| m.machine_id == r.machine_id && m.remote_id == r.remote_id);
        match row {
            None => Some("that remote question is gone"),
            Some(m) if m.qkey != r.qkey || m.db_id != r.db_id => {
                Some("that question changed — reopen it")
            }
            Some(_) => None,
        }
    }

    /// The question no longer answerable here: an answer or a dismissal is
    /// queued for it (Python `remote._busy_ids`); its group's row goes when
    /// it was the last.
    pub(crate) fn mark_busy(&mut self, r: &RemoteRef) {
        for g in &mut self.groups {
            g.questions
                .retain(|q| !(q.machine_id == r.machine_id && q.question.id == r.remote_id));
        }
        self.groups.retain(|g| !g.questions.is_empty());
    }
}

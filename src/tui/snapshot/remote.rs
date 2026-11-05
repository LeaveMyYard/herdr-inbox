//! The remote machines' mirror as the popup model reads it ([`Remote`],
//! Python `remote.open_groups`, `open_alerts`, `outbox_rows`, `problem_rows`,
//! `as_question`, `as_alert`), with the wall-clock answers the model has no
//! clock for: which outbox rows are expired or clearable, when they expire and
//! which machines are offline. A row of the mirror passed validation when it
//! was pulled; one that still lacks its id is left out.

use std::time::SystemTime;

use inbox_core::popup::{
    Alert, Machine, OutboxRow, Problem, QOption, Question, Remote, RemoteAlert, RemoteGroup,
    RemoteQuestion, RemoteRef,
};
use inbox_core::popup_view::unix_seconds;
use serde_json::{Map, Value};

use crate::clock::local_hhmm;
use crate::remote::pyvalue::py_truthy;
use crate::remote::validate::py_int;
use crate::store::Store;
use crate::store::mirror::{self, iso};
use crate::store::question::normalize_options;

/// The mirror now. `Err`: the database failed.
pub fn read(store: &Store, wall: SystemTime) -> rusqlite::Result<Remote> {
    let mut remote = Remote::default();
    remote.machines = store
        .machines_by_label()?
        .iter()
        .map(|m| Machine {
            machine_id: m.machine_id.clone(),
            label: m.label.clone(),
            offline: m.offline_note(wall),
        })
        .collect();
    remote.groups = store
        .mirror_open_groups()?
        .into_iter()
        .map(|(key, m, qs)| RemoteGroup {
            key,
            questions: qs
                .iter()
                .filter_map(|q| question(&m.machine_id, q))
                .collect(),
            machine_id: m.machine_id,
        })
        .collect();
    remote.mirror = store
        .mirror_identities()?
        .into_iter()
        .map(|(machine_id, remote_id, db_id, qkey)| RemoteRef {
            machine_id,
            remote_id,
            db_id,
            qkey,
        })
        .collect();
    remote.alerts = store
        .mirror_open_alerts()?
        .iter()
        .filter_map(|(m, a)| alert(&m.machine_id, a))
        .collect();
    remote.outbox = store
        .outbox_rows()?
        .iter()
        .map(|o| outbox(store, o, wall))
        .collect::<rusqlite::Result<_>>()?;
    remote.problems = store.problem_rows()?.into_iter().map(problem).collect();
    Ok(remote)
}

fn text(data: &Map<String, Value>, key: &str) -> Option<String> {
    data.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A mirrored question as the views show it (Python `as_question`; never
/// stored): its id and group are the remote ones.
fn question(machine_id: &str, data: &Map<String, Value>) -> Option<RemoteQuestion> {
    let id = data.get("id").and_then(py_int)?;
    let options = data
        .get("options")
        .filter(|o| py_truthy(o))
        .and_then(|o| normalize_options(o).ok().flatten())
        .unwrap_or_default();
    let flag = |key| data.get(key).is_some_and(py_truthy);
    Some(RemoteQuestion {
        machine_id: machine_id.to_string(),
        db_id: text(data, "_db_id").unwrap_or_default(),
        qkey: text(data, "qkey").unwrap_or_default(),
        question: Question {
            id,
            group: data.get("group").and_then(py_int).unwrap_or(id),
            status: text(data, "status").unwrap_or_default(),
            pane_id: text(data, "pane_id"),
            options: options
                .into_iter()
                .map(|o| QOption {
                    label: o.label,
                    description: o.description,
                    preview: o.preview,
                })
                .collect(),
            multi_select: flag("multi_select"),
            delivery_error: text(data, "delivery_error"),
            delivered: flag("delivered_at"),
            tab_label: text(data, "tab_label"),
            asked_by: text(data, "asked_by"),
            header: text(data, "header"),
            question: text(data, "question").unwrap_or_default(),
            group_title: text(data, "title"),
            answer: text(data, "answer"),
            created_at: text(data, "created_at").unwrap_or_default(),
            ..Question::default()
        },
    })
}

/// A mirrored alert as the views show it (Python `as_alert`).
fn alert(machine_id: &str, data: &Map<String, Value>) -> Option<RemoteAlert> {
    let or = |key, default: &str| {
        text(data, key)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default.to_string())
    };
    Some(RemoteAlert {
        machine_id: machine_id.to_string(),
        db_id: text(data, "_db_id").unwrap_or_default(),
        akey: text(data, "akey").unwrap_or_default(),
        alert: Alert {
            id: data.get("id").and_then(py_int)?,
            kind: or("kind", "agent"),
            status: or("status", "open"),
            pane_id: text(data, "pane_id"),
            severity: or("severity", "warning"),
            title: or("title", ""),
            message: text(data, "message"),
            tab_label: text(data, "tab_label"),
            asked_by: text(data, "asked_by"),
            created_at: text(data, "created_at").unwrap_or_default(),
        },
    })
}

fn outbox(store: &Store, o: &mirror::OutboxRow, wall: SystemTime) -> rusqlite::Result<OutboxRow> {
    let queued = o.state == "queued";
    Ok(OutboxRow {
        request_id: o.request_id.clone(),
        machine_id: o.machine_id.clone(),
        action: o.action.clone(),
        remote_id: o.remote_id,
        state: o.state.clone(),
        result: o.result.clone(),
        sending: queued && o.sending_at.as_deref().is_some_and(|s| !s.is_empty()),
        expired: queued && o.expires_at <= iso(wall),
        clearable: store.clearable(&o.request_id, &mut || wall)?,
        expires: unix_seconds(&o.expires_at)
            .map(local_hhmm)
            .unwrap_or_default(),
    })
}

fn problem(p: mirror::Problem) -> Problem {
    let field = |key| p.data.get(key).and_then(Value::as_str).map(str::to_string);
    Problem {
        question: field("question").unwrap_or_default(),
        answer: field("answer"),
        machine_id: p.machine_id,
        remote_id: p.remote_id,
        label: p.label,
        why: p.why,
    }
}

#[cfg(test)]
mod tests;

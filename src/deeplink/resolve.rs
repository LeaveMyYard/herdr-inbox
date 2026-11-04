//! The open group a request names (Python `deeplink.resolve`,
//! `_oldest_group`, `_resolve_remote`).

use herdr_agent_kit::agent_session::SessionId;
use serde_json::Value;

use super::Request;
use crate::cli::args::{pyint, question_id};
use crate::store::{Question, Store};
use crate::text::py_isspace;

/// What a request opens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A local group (its id).
    Group(i64),
    /// A remote machine's mirrored group (its key, Python `("remote", key)`).
    Remote(String),
}

/// A saved herdr machine that is enabled: its label and id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineName {
    /// Its label.
    pub label: Option<String>,
    /// Its id.
    pub id: Option<String>,
}

/// The open group `request` names and the question to show first, or
/// `None` when it is gone (or names nothing). A question names its group; a
/// pane or a session its oldest open group. A remote question
/// (`machine:Qn`) resolves only on an enabled machine, which `machines`
/// lists (asked only then).
pub fn resolve(
    store: &Store,
    request: &Request,
    machines: impl FnOnce() -> Vec<MachineName>,
) -> rusqlite::Result<Option<(Target, Option<i64>)>> {
    let value = request.value.trim_matches(py_isspace);
    let kind = request.kind.as_str();
    if kind == "question" && value.contains(':') {
        return remote(store, value, machines);
    }
    let open = store.list(false)?;
    Ok(match kind {
        "question" => question_id(value).and_then(|n| {
            let q = open.iter().find(|q| q.id == n)?;
            Some((Target::Group(q.group()), Some(q.id)))
        }),
        "group" => pyint::parse(value)
            .filter(|n| open.iter().any(|q| q.group() == *n))
            .map(|n| (Target::Group(n), None)),
        "pane" => oldest(open.iter().filter(|q| q.pane_id.as_deref() == Some(value))),
        "session" => match SessionId::parse(value) {
            Some(sid) => oldest(open.iter().filter(|q| {
                q.session_id.as_deref().and_then(SessionId::parse).as_ref() == Some(&sid)
            })),
            None => None,
        },
        _ => None,
    })
}

fn oldest<'a>(qs: impl Iterator<Item = &'a Question>) -> Option<(Target, Option<i64>)> {
    let q = qs.min_by_key(|q| q.id)?;
    Some((Target::Group(q.group()), None))
}

/// `machine:Qn`: the open mirrored group holding that question, if
/// `machine` (the part before the first colon) is an enabled machine's
/// label or id.
fn remote(
    store: &Store,
    value: &str,
    machines: impl FnOnce() -> Vec<MachineName>,
) -> rusqlite::Result<Option<(Target, Option<i64>)>> {
    let (label, reference) = value.split_once(':').unwrap_or((value, ""));
    let named =
        |m: &MachineName| m.label.as_deref() == Some(label) || m.id.as_deref() == Some(label);
    if !machines().iter().any(named) {
        return Ok(None);
    }
    let Some(n) = question_id(reference) else {
        return Ok(None);
    };
    for (key, m, qs) in store.mirror_open_groups()? {
        let ours = m.label.as_deref() == Some(label) || m.machine_id == label;
        if ours
            && qs
                .iter()
                .any(|q| q.get("id").and_then(Value::as_i64) == Some(n))
        {
            return Ok(Some((Target::Remote(key), None)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;

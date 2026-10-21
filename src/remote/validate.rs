//! A remote's export, validated and capped (Python `remote.validate_export`,
//! `clean_row`, `clean_alerts`, `_text`). Remote output is untrusted: a row
//! that does not conform is dropped, a header that does not is an error.
//!
//! Python's types are kept where they decide anything: a JSON `true`/`false`
//! is an `int` to Python's `isinstance`, so it passes an integer check (as 1 or
//! 0) and is kept as it came in the stored row.
//!
//! Where Rust reads an export differently (none of it comes from an inbox's
//! own export; each fails safe):
//! - an escaped lone surrogate reads as U+FFFD ([`crate::pyjson::loads`]);
//! - `NaN`, `Infinity` and numbers beyond f64 fail the whole export (Python
//!   reads them);
//! - JSON nested deeper than 128 fails the whole export (serde_json's stack
//!   guard); Python reads it and ignores a field it does not use;
//! - an integer beyond i64 in `id`, `group`, `seq`, `ids` or `gone` drops the
//!   row or refuses the export, recorded as a failed pull with backoff; Python
//!   takes it and then fails at SQLite inside the apply (`OverflowError`), out
//!   of `pull` without `_fail`;
//! - a non-string `version` is stored as its JSON text (Python: its `str()`,
//!   e.g. `['a']`); only `remote_machines.version` shows it;
//! - a failed JSON parse records serde's message as `last_error`, not Python's.

use serde_json::{Map, Value};

use super::fields::is_hex_key;
pub use super::fields::{NotAString, text};
pub use super::pyvalue::py_int;
use super::pyvalue::{py_int_strict, py_repr, py_repr_str, py_truthy};
use crate::pyjson;

/// The export format this puller speaks (Python `EXPORT_FORMAT`).
pub const EXPORT_FORMAT: i64 = 1;
/// The largest export read, in bytes (Python `MAX_EXPORT_BYTES`, on the text).
pub const MAX_EXPORT_BYTES: usize = 2_000_000;
/// The most questions (and alerts, and gone ids) kept from one export.
pub const MAX_QUESTIONS: usize = 1000;
/// Each text field of a row and its cap in characters, in the stored order (Python `CAPS`).
pub const CAPS: [(&str, usize); 15] = [
    ("question", 4000),
    ("header", 60),
    ("tab_label", 120),
    ("pane_id", 64),
    ("asked_by", 120),
    ("title", 120),
    ("answer", 4000),
    ("delivery_error", 300),
    ("status", 20),
    ("created_at", 40),
    ("answered_at", 40),
    ("delivered_at", 40),
    ("verified_at", 40),
    ("acknowledged_at", 40),
    ("qkey", 64),
];
/// The statuses a row may have (Python `STATUSES`).
pub const STATUSES: [&str; 4] = ["open", "answered", "withdrawn", "dismissed"];

/// An export, validated.
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    /// The remote database's identity.
    pub db_id: String,
    /// Its change cursor.
    pub seq: i64,
    /// A full snapshot (`ids` lists every live question).
    pub full: bool,
    /// The remote's version, as text.
    pub version: Option<String>,
    /// Every live question's id (a full snapshot).
    pub ids: Vec<i64>,
    /// Questions changed and no longer there.
    pub gone: Vec<i64>,
    /// The rows, cleaned: stored as they are (`data_json`).
    pub questions: Vec<Map<String, Value>>,
    /// The alerts, cleaned (`None` from a remote older than alerts).
    pub alerts: Option<Vec<Map<String, Value>>>,
}

/// Why an export was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// Another export format: the remote needs an upgrade (Python `PermissionError`).
    Format(String),
    /// Malformed (Python `ValueError`).
    Bad(String),
}

fn clean_option(o: &Value) -> Result<Value, NotAString> {
    let mut out = Map::new();
    match o {
        Value::String(_) => {
            out.insert("label".into(), text(Some(o), 200)?.into());
        }
        Value::Object(m) if matches!(m.get("label"), Some(Value::String(_))) => {
            out.insert("label".into(), text(m.get("label"), 200)?.into());
            for (k, cap) in [("description", 500), ("preview", 4000)] {
                if let Some(v @ Value::String(_)) = m.get(k) {
                    out.insert(k.into(), text(Some(v), cap)?.into());
                }
            }
        }
        _ => return Err(NotAString),
    }
    Ok(Value::Object(out))
}

/// One exported question, validated and capped; `None` if it does not conform.
pub fn clean_row(row: &Value) -> Option<Map<String, Value>> {
    let m = row.as_object()?;
    let (id, group) = (m.get("id")?, m.get("group")?);
    py_int(id)?;
    py_int(group)?;
    let mut out = Map::new();
    out.insert("id".into(), id.clone());
    out.insert("group".into(), group.clone());
    for (key, cap) in CAPS {
        out.insert(
            key.into(),
            text(m.get(key), cap)
                .ok()?
                .map_or(Value::Null, Value::String),
        );
    }
    let s = |k: &str| out.get(k).and_then(Value::as_str);
    if !s("status").is_some_and(|st| STATUSES.contains(&st))
        || !s("qkey").is_some_and(is_hex_key)
        || s("question").is_none_or(str::is_empty)
    {
        return None;
    }
    let opts = match m.get("options") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) if !py_truthy(v) => Vec::new(),
        Some(Value::Array(list)) if list.len() <= 9 => list
            .iter()
            .map(clean_option)
            .collect::<Result<_, _>>()
            .ok()?,
        Some(_) => return None,
    };
    out.insert("options".into(), Value::Array(opts));
    for key in ["multi_select", "hidden", "agent_gone"] {
        out.insert(key.into(), Value::Bool(m.get(key).is_some_and(py_truthy)));
    }
    let attempts = m
        .get("delivery_attempts")
        .filter(|a| py_int(a).is_some_and(|n| (0..10_000).contains(&n)));
    out.insert(
        "delivery_attempts".into(),
        attempts.cloned().unwrap_or(Value::from(0)),
    );
    Some(out)
}

/// A remote's alerts, whole on every pull; malformed ones are skipped.
pub fn clean_alerts(raw: Option<&Value>) -> Result<Option<Vec<Map<String, Value>>>, ExportError> {
    let list = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(ExportError::Bad("bad alerts".into())),
    };
    let mut out = Vec::new();
    for a in list.iter().take(MAX_QUESTIONS) {
        let Some(m) = a.as_object() else { continue };
        let (Some(id), Some(Value::String(akey))) = (m.get("id"), m.get("akey")) else {
            continue;
        };
        let status = m.get("status").and_then(Value::as_str);
        if py_int(id).is_none()
            || !is_hex_key(akey)
            || !status.is_some_and(|s| ["open", "acked", "withdrawn", "cleared"].contains(&s))
        {
            continue;
        }
        let field = |k: &str, cap: usize| {
            text(m.get(k), cap).map_err(|NotAString| ExportError::Bad("not a string".into()))
        };
        let kind = if m.get("kind").and_then(Value::as_str) == Some("blocked") {
            "blocked"
        } else {
            "agent"
        };
        let severity = m
            .get("severity")
            .and_then(Value::as_str)
            .filter(|s| ["info", "warning", "urgent"].contains(s));
        let title = field("title", 200)?
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "(no title)".into());
        let mut o = Map::new();
        o.insert("id".into(), id.clone());
        o.insert("akey".into(), akey.clone().into());
        o.insert("status".into(), status.unwrap_or_default().into());
        o.insert("kind".into(), kind.into());
        o.insert("severity".into(), severity.unwrap_or("warning").into());
        o.insert("title".into(), title.into());
        for (k, cap) in [
            ("message", 2000),
            ("tab_label", 120),
            ("pane_id", 80),
            ("asked_by", 120),
            ("created_at", 40),
        ] {
            o.insert(k.into(), field(k, cap)?.map_or(Value::Null, Value::String));
        }
        out.push(o);
    }
    Ok(Some(out))
}

fn ints(v: &Value) -> Option<Vec<i64>> {
    v.as_array()?.iter().map(py_int).collect()
}

/// What `inbox export` printed, validated (Python `validate_export`).
pub fn validate_export(raw: &str) -> Result<Export, ExportError> {
    let bad = |m: &str| ExportError::Bad(m.into());
    if raw.chars().count() > MAX_EXPORT_BYTES {
        return Err(bad("export too large"));
    }
    let data = pyjson::loads(raw).map_err(|e| bad(&e.to_string()))?;
    let m = data.as_object().ok_or_else(|| bad("not an object"))?;
    if m.get("format").and_then(py_int_strict) != Some(EXPORT_FORMAT) {
        let version = m.get("version").map_or("?".to_string(), py_repr_str);
        let format = m.get("format").map_or("None".to_string(), py_repr);
        return Err(ExportError::Format(format!(
            "inbox {version} speaks export format {format}"
        )));
    }
    let db_id = m
        .get("db_id")
        .and_then(Value::as_str)
        .filter(|d| is_hex_key(d))
        .ok_or_else(|| bad("bad header"))?;
    let seq = m
        .get("seq")
        .and_then(py_int)
        .ok_or_else(|| bad("bad header"))?;
    let qs = m
        .get("questions")
        .and_then(Value::as_array)
        .filter(|q| q.len() <= MAX_QUESTIONS)
        .ok_or_else(|| bad("bad questions"))?;
    let questions = qs.iter().filter_map(clean_row).collect();
    let full = m.get("full").is_some_and(py_truthy);
    let ids = match m.get("ids") {
        Some(v) if full => Some(ints(v).ok_or_else(|| bad("full snapshot without ids"))?),
        _ if full => return Err(bad("full snapshot without ids")),
        Some(v) if py_truthy(v) => ints(v),
        _ => Some(Vec::new()),
    };
    let gone = match m.get("gone") {
        Some(v) if py_truthy(v) => ints(v).ok_or_else(|| bad("bad gone list"))?,
        _ => Vec::new(),
    };
    let version = m.get("version").map_or("?".to_string(), py_repr_str);
    Ok(Export {
        db_id: db_id.into(),
        seq,
        full,
        version: text(Some(&Value::String(version)), 20).unwrap_or(None),
        ids: ids.unwrap_or_default(),
        gone: gone.into_iter().take(MAX_QUESTIONS).collect(),
        questions,
        alerts: clean_alerts(m.get("alerts"))?,
    })
}

#[cfg(test)]
mod tests;

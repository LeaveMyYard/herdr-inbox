//! The hook payload Claude Code writes to stdin, read as Python reads its
//! dict: a missing key, `null` and a falsy value all mean "absent".

use herdr_agent_kit::agent_session::claude::wire::Line;
use serde_json::{Map, Value};

mod deep;

/// The fields the inbox's hooks read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Payload {
    /// `session_id`.
    pub session_id: Option<String>,
    /// `transcript_path`.
    pub transcript_path: Option<String>,
    /// `source` (SessionStart: `startup`, `resume`, `clear`, `compact`, `fork`).
    pub source: Option<String>,
    /// `agent_id` is present and truthy: a subagent's or teammate's hook.
    pub agent_id: bool,
}

impl Payload {
    /// The payload from stdin's text: empty or blank text is `{}`. Parsed
    /// as Python's `json.loads` would (the kit's transcript-line parser): a
    /// lone UTF-16 surrogate escape, as a string cut inside an emoji in
    /// `tool_response` carries, does not reject it. Valid JSON that is not an
    /// object reads as `{}` (Python's hook then fell back to the bare rules at
    /// SessionStart and did nothing else). An object nested deeper than
    /// serde_json's 128 levels (a deep `tool_response`) is still read
    /// ([`deep`]). `None` when it is not JSON (the hook then does nothing).
    pub fn parse(raw: &str) -> Option<Payload> {
        if raw.trim().is_empty() {
            return Some(Payload::default());
        }
        let map = match Line::parse(raw.trim().as_bytes()) {
            Some(line) => line.into_map(),
            None => match serde_json::from_str::<Value>(raw) {
                Ok(_) => return Some(Payload::default()), // valid, not an object
                Err(e) if deep::too_deep(&e) => deep::top_level(raw)?,
                Err(_) => return None,
            },
        };
        let map = &map;
        Some(Payload {
            session_id: text(map, "session_id"),
            transcript_path: text(map, "transcript_path"),
            source: text(map, "source"),
            agent_id: map.get("agent_id").is_some_and(truthy),
        })
    }
}

/// A non-empty string field. Claude Code writes these as strings; any other
/// type is treated as absent, and so is one that held a lone surrogate
/// (read as U+FFFD): Python could not write it to SQLite and raised before
/// writing anything.
fn text(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .filter(|v| !v.as_str().is_some_and(|s| s.contains('\u{FFFD}')))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Python's truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests;

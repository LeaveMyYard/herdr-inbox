//! The hook payload Claude Code writes to stdin, read as Python reads its
//! dict: a missing key, `null` and a falsy value all mean "absent".

use serde_json::{Map, Value};

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
    /// The payload from stdin's text: empty or blank text is `{}`; `None`
    /// when it is not a JSON object (the hook then does nothing).
    pub fn parse(raw: &str) -> Option<Payload> {
        if raw.trim().is_empty() {
            return Some(Payload::default());
        }
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) else {
            return None;
        };
        Some(Payload {
            session_id: text(&map, "session_id"),
            transcript_path: text(&map, "transcript_path"),
            source: text(&map, "source"),
            agent_id: map.get("agent_id").is_some_and(truthy),
        })
    }
}

/// A non-empty string field. Claude Code writes these as strings; any other
/// type is treated as absent.
fn text(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
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

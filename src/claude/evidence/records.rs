//! The transcript records the evidence index reads (Python
//! `transcript._tool_uses`, `_tool_results`, `_answers_in`): the
//! conversation's own inbox calls and what came back to them.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::shell::own_answer_qids;

/// What an inbox call of the conversation did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Use {
    /// A Bash `inbox answer <n>`: the conversation answers these itself.
    Own(BTreeSet<i64>),
    /// herdr-inbox's `check_answers` / `list_my_questions` MCP tool.
    Read,
}

const INBOX_READS: [&str; 2] = ["check_answers", "list_my_questions"];

fn content(record: &Map<String, Value>) -> Option<&Vec<Value>> {
    // Python `(record.get("message") or {}).get("content")`
    record
        .get("message")?
        .as_object()?
        .get("content")?
        .as_array()
}

/// `(tool_use id, what)` of the inbox calls an assistant record makes.
pub fn tool_uses(record: &Map<String, Value>) -> Vec<(String, Use)> {
    if record.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for block in content(record).into_iter().flatten() {
        let Some(block) = block.as_object() else {
            continue;
        };
        let (Some("tool_use"), Some(id)) = (
            block.get("type").and_then(Value::as_str),
            block.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        let command = block
            .get("input")
            .and_then(Value::as_object)
            .and_then(|i| i.get("command"))
            .and_then(Value::as_str);
        if name == "Bash" && command.is_some_and(|c| c.contains("inbox")) {
            let qids = own_answer_qids(command.unwrap_or_default());
            if !qids.is_empty() {
                out.push((id.to_string(), Use::Own(qids)));
            }
        } else if name.contains("herdr-inbox") && INBOX_READS.iter().any(|r| name.ends_with(r)) {
            out.push((id.to_string(), Use::Read));
        }
    }
    out
}

/// `(tool_use_id, text)` of the tool results a user record carries.
pub fn tool_results(record: &Map<String, Value>) -> Vec<(Option<String>, String)> {
    if record.get("type").and_then(Value::as_str) != Some("user") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for block in content(record).into_iter().flatten() {
        let Some(block) = block.as_object() else {
            continue;
        };
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let used = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(String::from);
        match block.get("content") {
            Some(Value::String(s)) => out.push((used, s.clone())),
            Some(Value::Array(items)) => {
                for b in items {
                    if let Some(t) = b
                        .as_object()
                        .and_then(|b| b.get("text"))
                        .and_then(Value::as_str)
                    {
                        out.push((used.clone(), t.to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Python `isinstance(v, int)`: an integer, or a boolean (Python's `bool`
/// is an `int`).
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// Every `{"id": <int>, "answer": "<str>"}` object anywhere in `value`, an
/// object before what it holds.
pub fn answers_in(value: &Value, out: &mut Vec<(i64, String)>) {
    match value {
        Value::Object(map) => {
            if let (Some(id), Some(Value::String(answer))) =
                (map.get("id").and_then(py_int), map.get("answer"))
            {
                out.push((id, answer.clone()));
            }
            for v in map.values() {
                answers_in(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                answers_in(v, out);
            }
        }
        _ => {}
    }
}

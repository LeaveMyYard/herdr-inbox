//! A row's revision token (`app-feed-v1.md`, "Tokens"). **SAFETY**: a verb's
//! write stands on this hash, so every field here is frozen for v1: a field
//! added to a row later stays out of it, or a writer and a verb of two
//! versions would refuse each other's rows.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const GROUP: [&str; 6] = ["key", "group", "title", "created_at", "asker", "questions"];
const FAILED: [&str; 10] = [
    "key",
    "question_id",
    "group",
    "header",
    "question",
    "asked_by",
    "answer",
    "answered_at",
    "retries",
    "asker",
];
const ALERT: [&str; 9] = [
    "key",
    "id",
    "alert_kind",
    "severity",
    "title",
    "message",
    "asked_by",
    "created_at",
    "asker",
];
const ASKER: [&str; 5] = ["host", "pane", "terminal", "tab_label", "cwd"];
const QUESTION: [&str; 8] = [
    "id",
    "header",
    "question",
    "options",
    "multi_select",
    "context",
    "asked_by",
    "draft",
];
const OPTION: [&str; 3] = ["label", "description", "preview"];
const DRAFT: [&str; 3] = ["choices", "notes", "text"];

/// The token of `entry`, a `group`, `failed` or `alert` row with every field
/// its kind has. `None` for any other kind: those take no verb.
pub fn token(entry: &Value) -> Option<String> {
    let fields: &[&str] = match entry.get("kind")?.as_str()? {
        "group" => &GROUP,
        "failed" => &FAILED,
        "alert" => &ALERT,
        _ => return None,
    };
    let canonical = canonical(&input(entry, fields)).to_string();
    let hash = Sha256::digest(canonical.as_bytes());
    Some(hash[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// The fields of `entry` that count, each reduced to its own frozen list.
fn input(entry: &Value, fields: &[&str]) -> Value {
    let mut picked = pick(entry, fields);
    if let Some(asker) = picked.get("asker") {
        picked["asker"] = pick(asker, &ASKER);
    }
    if let Some(questions) = picked.get("questions").and_then(Value::as_array) {
        picked["questions"] = questions.iter().map(question).collect();
    }
    picked
}

fn question(q: &Value) -> Value {
    let mut picked = pick(q, &QUESTION);
    if let Some(options) = picked.get("options").and_then(Value::as_array) {
        picked["options"] = options.iter().map(|o| pick(o, &OPTION)).collect();
    }
    if picked.get("draft").is_some_and(Value::is_object) {
        picked["draft"] = pick(&picked["draft"], &DRAFT);
    }
    picked
}

fn pick(object: &Value, fields: &[&str]) -> Value {
    let picked: Map<String, Value> = fields
        .iter()
        .filter_map(|f| Some(((*f).to_string(), object.get(*f)?.clone())))
        .collect();
    Value::Object(picked)
}

/// `v` with the keys of every object in code point order. Compact JSON of the
/// result is Python's `json.dumps(v, sort_keys=True, separators=(",", ":"),
/// ensure_ascii=False)`: UTF-8 orders as code points do.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            let sorted: Map<String, Value> = keys
                .into_iter()
                .map(|k| (k.clone(), canonical(&map[k])))
                .collect();
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests;

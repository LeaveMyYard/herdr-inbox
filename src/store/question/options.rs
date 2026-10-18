//! A question's options (Python `store.normalize_options`): plain strings or
//! `{label, description, preview}` objects, read and written as Python does.

use serde_json::{Map, Value};

use crate::pyjson::dumps;

/// One option.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QOption {
    /// What the operator picks.
    pub label: String,
    /// More words about it (an empty one is none).
    pub description: Option<String>,
    /// A preview (an empty one is none).
    pub preview: Option<String>,
}

/// The options of a JSON list (strings or objects with a `label`); `None` for
/// an empty list or anything that is not a list.
pub fn normalize_options(value: &Value) -> Option<Vec<QOption>> {
    let items = value.as_array().filter(|a| !a.is_empty())?;
    Some(
        items
            .iter()
            .map(|o| match o {
                Value::String(s) => QOption {
                    label: s.clone(),
                    description: None,
                    preview: None,
                },
                other => QOption {
                    label: text(other.get("label")).unwrap_or_default(),
                    description: text(other.get("description")),
                    preview: text(other.get("preview")),
                },
            })
            .collect(),
    )
}

/// A string field (Python's `str(o["label"])` / `o.get(...) or None`): empty
/// and missing are `None`; a non-string is its JSON text.
fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// `question_ext.options_json`: the full objects, as Python dumps them.
pub fn options_json(options: &[QOption]) -> String {
    let list: Vec<Value> = options
        .iter()
        .map(|o| {
            let mut m = Map::new();
            m.insert("label".into(), Value::String(o.label.clone()));
            m.insert(
                "description".into(),
                o.description.clone().map_or(Value::Null, Value::String),
            );
            m.insert(
                "preview".into(),
                o.preview.clone().map_or(Value::Null, Value::String),
            );
            Value::Object(m)
        })
        .collect();
    dumps(&Value::Array(list))
}

/// `questions.options`: the labels only (what 0.1.x reads).
pub fn labels_json(options: &[QOption]) -> String {
    dumps(&Value::Array(
        options
            .iter()
            .map(|o| Value::String(o.label.clone()))
            .collect(),
    ))
}

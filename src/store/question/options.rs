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

/// What Python's `normalize_options` raises on: a mapping without `label`
/// (`KeyError`), an item or value that is neither (`TypeError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsError {
    /// An object without `label`.
    NoLabel,
    /// An item that is not a string or an object, or a scalar where a list was expected.
    NotOptions,
}

impl std::fmt::Display for OptionsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OptionsError::NoLabel => "an option without a label",
            OptionsError::NotOptions => "options that are not strings or objects",
        })
    }
}

impl std::error::Error for OptionsError {}

/// Python `store.normalize_options`, input for input (vectors captured from
/// Python in `tests/fixtures/store-questions.json`): a falsy value is `None`; a
/// list gives one option per string or `{label, description, preview}` object;
/// iterating a string gives its characters, an object its keys; anything else
/// raises. `label` is `str(label)`; `description` and `preview` are kept when
/// truthy. Two results cannot be Python's: a list or object `label`,
/// `description` or `preview` is its JSON text (Python: its `repr`), and a
/// truthy non-string `description` or `preview` is a string spelled as `str()`
/// would (Python keeps the value itself). The MCP schema allows strings only,
/// so neither comes from an agent.
pub fn normalize_options(value: &Value) -> Result<Option<Vec<QOption>>, OptionsError> {
    if !truthy(value) {
        return Ok(None);
    }
    let labels = |labels: Vec<String>| labels.into_iter().map(plain).collect();
    let out = match value {
        Value::Array(items) => items.iter().map(item).collect::<Result<_, _>>()?,
        Value::String(s) => labels(s.chars().map(String::from).collect()),
        Value::Object(m) => labels(m.keys().cloned().collect()),
        _ => return Err(OptionsError::NotOptions),
    };
    Ok(Some(out))
}

fn plain(label: String) -> QOption {
    QOption {
        label,
        description: None,
        preview: None,
    }
}

fn item(o: &Value) -> Result<QOption, OptionsError> {
    match o {
        Value::String(s) => Ok(plain(s.clone())),
        Value::Object(m) => Ok(QOption {
            label: py_str(m.get("label").ok_or(OptionsError::NoLabel)?),
            description: m.get("description").filter(|v| truthy(v)).map(py_str),
            preview: m.get("preview").filter(|v| truthy(v)).map(py_str),
        }),
        _ => Err(OptionsError::NotOptions),
    }
}

/// Python's truth value of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// Python's `str()` of a JSON value (see [`normalize_options`] for lists and objects).
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => dumps(other),
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

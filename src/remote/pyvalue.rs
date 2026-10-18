//! Python's view of JSON values where a remote's export depends on it:
//! `isinstance(v, int)` (a bool is one), truthiness, `==` with an int, and
//! `str()`/`repr()` for messages.

use serde_json::Value;

use crate::pyjson;

/// Python's `isinstance(v, int)`: an integer, or a bool (1 or 0).
pub fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// Python's truth value of a JSON value.
pub fn py_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `data.get("format") != 1` in Python: `1`, `1.0` and `true` equal 1.
pub fn py_int_strict(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// Python's `str()` of a JSON value, for messages.
pub fn py_repr_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => py_repr(other),
    }
}

/// Python's `repr()` of a JSON scalar (`'x'`, `None`, `True`), JSON text otherwise.
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{s}'"),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => pyjson::dumps(other),
    }
}

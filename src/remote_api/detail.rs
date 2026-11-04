//! The answer an `inbox apply` request carries (Python
//! `service.make_detail(detail.get("text"), detail.get("choices"),
//! detail.get("notes"))`, then `format_answer`), read with Python's types.

use inbox_core::popup::Detail;
use inbox_core::popup_view::format_answer;
use serde_json::{Map, Value, json};

use crate::remote::pyvalue::py_truthy;

/// A detail Python's `make_detail` or `format_answer` raises on (a
/// traceback there): a non-string text, picked label or note, choices that
/// are not iterable, notes that are not an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

/// The answer's one line and its detail as `answer_json` stores it
/// (`{"choices", "notes", "text"}`); `None` when nothing is picked and the
/// text is blank (Python: `make_detail` returns `None`).
///
/// Python's iteration rules hold: `choices` given as a string is its
/// characters, as an object its keys; a falsy value is none.
pub fn answer(detail: &Map<String, Value>) -> Result<Option<(String, Value)>, Malformed> {
    let text = match detail.get("text") {
        Some(v) if py_truthy(v) => Some(v.as_str().ok_or(Malformed)?),
        _ => None,
    };
    let choices: Vec<String> = match detail.get("choices") {
        Some(v) if !py_truthy(v) => Vec::new(),
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|c| c.as_str().map(str::to_string).ok_or(Malformed))
            .collect::<Result<_, _>>()?,
        Some(Value::String(s)) => s.chars().map(String::from).collect(),
        Some(Value::Object(m)) => m.keys().cloned().collect(),
        Some(_) => return Err(Malformed), // list(5): not iterable
    };
    let mut notes = Vec::new();
    match detail.get("notes") {
        Some(v) if !py_truthy(v) => {}
        None => {}
        Some(Value::Object(m)) => {
            for (k, v) in m {
                // `k in choices and v and v.strip()`: only a picked label's truthy note is read
                if choices.contains(k) && py_truthy(v) {
                    notes.push((k.clone(), v.as_str().ok_or(Malformed)?.to_string()));
                }
            }
        }
        Some(_) => return Err(Malformed), // no .items()
    }
    let Some(d) = Detail::make(text, choices, notes) else {
        return Ok(None);
    };
    let line = format_answer(&d);
    let notes: Map<String, Value> = d
        .notes
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    Ok(Some((
        line,
        json!({"choices": d.choices, "notes": notes, "text": d.text}),
    )))
}

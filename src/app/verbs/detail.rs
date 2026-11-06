//! `--detail`: an answer in parts. It comes out as the popup would store it
//! (`Detail::make`), with its choices in option order. **SAFETY**: this is
//! what the question's answer will say.

use inbox_core::popup::Detail;
use serde_json::Value;

use crate::store::question::Question;
use crate::tui::snapshot::convert::detail_json;

/// The largest `--detail`, in bytes.
pub const MAX_DETAIL: usize = 64 * 1024;

/// `v` is a Detail: exactly its three fields, of their types. An unknown field
/// would be an operator's words silently dropped.
pub fn well_formed(v: &Value) -> bool {
    let Some(map) = v.as_object() else {
        return false;
    };
    let choices = map
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|c| c.iter().all(Value::is_string));
    let notes = map
        .get("notes")
        .and_then(Value::as_object)
        .is_some_and(|n| n.values().all(Value::is_string));
    let text = map
        .get("text")
        .is_some_and(|t| t.is_null() || t.is_string());
    map.len() == 3 && choices && notes && text
}

/// Why a detail does not fit its question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unfit(pub String);

/// The stored form of `detail` for `q`, `None` when it normalises to nothing.
/// `detail` is [`well_formed`].
pub fn normalise(q: &Question, detail: &Value) -> Result<Option<Value>, Unfit> {
    let picked: Vec<&str> = detail["choices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let labels: Vec<&str> = q
        .options
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    for (i, choice) in picked.iter().enumerate() {
        if !labels.contains(choice) {
            return Err(Unfit(format!("Q{} has no such option: {choice}", q.id)));
        }
        if picked[..i].contains(choice) {
            return Err(Unfit(format!("Q{} has a choice twice", q.id)));
        }
    }
    if !q.multi_select && picked.len() > 1 {
        return Err(Unfit(format!("Q{} takes one choice", q.id)));
    }
    // a label two options share means the first, so it comes out once, in its place
    let mut ordered: Vec<String> = Vec::new();
    for label in labels {
        if picked.contains(&label) && !ordered.iter().any(|l| l == label) {
            ordered.push(label.to_string());
        }
    }
    let notes = detail["notes"].as_object().into_iter().flatten();
    let notes = notes
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect();
    let made = Detail::make(detail["text"].as_str(), ordered, notes);
    Ok(made.as_ref().map(detail_json))
}

#[cfg(test)]
mod tests;

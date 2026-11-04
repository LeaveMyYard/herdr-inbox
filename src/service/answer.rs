//! Answering and closing questions (Python `service.make_detail`,
//! `format_answer`, `submit`, `answer`, `close`). **SAFETY**: this decides
//! which answer a question gets and hands it to delivery.

use inbox_core::popup::Detail;
use inbox_core::popup_view::format_answer;
use serde_json::{Map, Value, json};

use super::Delivery;
use crate::pyjson::{dumps, lossless};
use crate::store::{Question, Store};
use crate::surface::markers::Markers;

/// The detail of a typed answer (Python `make_detail(text)`): free text only,
/// stripped; `None` when it is blank.
pub fn text_detail(text: &str) -> Option<Value> {
    let d = Detail::make(Some(text), Vec::new(), Vec::new())?;
    Some(json!({"choices": [], "notes": {}, "text": d.text}))
}

/// The one-line answer a stored detail reads as (Python `format_answer(d.get("choices")
/// or [], d.get("notes"), d.get("text"))`): picked labels with their notes, then the text.
pub fn answer_text(d: &Value) -> String {
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let empty = Map::new();
    let notes = d.get("notes").and_then(Value::as_object).unwrap_or(&empty);
    format_answer(&Detail {
        choices: strings(d.get("choices")),
        notes: notes
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect(),
        text: d.get("text").and_then(Value::as_str).map(str::to_string),
    })
}

/// Sends the drafted answers among `qids` (Python `submit`); questions not
/// open or without a draft are skipped. Each answer is its draft, formatted,
/// and the draft goes into `answer_json` as Python writes it back (never the
/// lossy decoded form: see [`lossless`]). With `deliver`, the
/// answered ones go to [`Delivery::deliver`] in one batch; without, they are
/// only recorded (the popup hands delivery to a detached process). Then
/// every pane they came from gets its marks refreshed. Returns them as they
/// stand after all that.
pub fn submit(
    store: &Store,
    qids: &[i64],
    deliver: Option<&mut dyn Delivery>,
    markers: &mut Markers<'_>,
) -> rusqlite::Result<Vec<Question>> {
    let mut answered = Vec::new();
    for &qid in qids {
        let Some(q) = store.get(qid)? else { continue };
        let Some(draft) = q.draft_json.as_ref().filter(|d| truthy(d)) else {
            continue;
        };
        if q.status != "open" {
            continue;
        }
        // Python writes dumps(loads(raw)): so does this, unless the read was lossy (a lone
        // surrogate), where the stored text is carried on as it is instead
        let detail = match store.draft_text(qid)? {
            Some(raw) if !lossless(&raw) => raw,
            _ => dumps(draft),
        };
        if store.set_answer_text(qid, &answer_text(draft), Some(&detail))? {
            answered.extend(store.get(qid)?);
        }
    }
    if let Some(d) = deliver {
        d.deliver(store, &answered)?;
    }
    let mut panes: Vec<Option<&str>> = Vec::new();
    for q in &answered {
        if !panes.contains(&q.pane_id.as_deref()) {
            panes.push(q.pane_id.as_deref());
        }
    }
    for pane in panes {
        markers.update(pane);
    }
    let mut out = Vec::new();
    for q in &answered {
        out.extend(store.get(q.id)?);
    }
    Ok(out)
}

/// Answers one open question right away with `text` (Python `answer`, the
/// CLI path): its draft, then [`submit`]. `None`: not open (or blank text).
pub fn answer(
    store: &Store,
    qid: i64,
    text: &str,
    deliver: Option<&mut dyn Delivery>,
    markers: &mut Markers<'_>,
) -> rusqlite::Result<Option<Question>> {
    let Some(detail) = text_detail(text) else {
        return Ok(None);
    };
    if !store.set_draft(qid, Some(&detail))? {
        return Ok(None);
    }
    Ok(submit(store, &[qid], deliver, markers)?.into_iter().next())
}

/// Closes an open question as `status` (`dismissed`, `withdrawn`) with an
/// optional `note`, and refreshes its pane's marks (Python `close`).
/// `false`: it is not open.
pub fn close(
    store: &Store,
    qid: i64,
    status: &str,
    note: Option<&str>,
    markers: &mut Markers<'_>,
) -> rusqlite::Result<bool> {
    let Some(q) = store.get(qid)? else {
        return Ok(false);
    };
    if !store.set_status(qid, status, note)? {
        return Ok(false);
    }
    markers.update(q.pane_id.as_deref());
    Ok(true)
}

/// Python's truthiness of a decoded JSON value.
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

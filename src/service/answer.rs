//! Answering and closing questions (Python `service.make_detail`,
//! `format_answer`, `submit`, `answer`, `close`). **SAFETY**: this decides
//! which answer a question gets and hands it to delivery.

use inbox_core::popup::Detail;
use inbox_core::popup_view::format_answer;
use serde_json::{Map, Value, json};

use super::Delivery;
use crate::pyjson::{dumps, loads, lossless};
use crate::remote::pyvalue::py_truthy;
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
/// and the draft goes into `answer_json` as Python writes it back. A draft
/// that cannot be read without loss (a lone surrogate, see [`lossless`]) is
/// an error, as Python's write raises there, and so is a draft that is no
/// JSON object: answers before it stay recorded, nothing is delivered or
/// marked. (Python raises on a lone surrogate only once it reaches the
/// answer text; this refuses it anywhere in the draft, which no popup
/// writes.) With `deliver`, the
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
    submit_marking(store, qids, deliver, &mut |pane| markers.update(pane))
}

/// [`submit`], each pane's marks refreshed by `marks` (the popup queues them
/// for its background thread, as Python's popup does with
/// `service.enable_background`).
pub fn submit_marking(
    store: &Store,
    qids: &[i64],
    deliver: Option<&mut dyn Delivery>,
    marks: &mut dyn FnMut(Option<&str>),
) -> rusqlite::Result<Vec<Question>> {
    let mut answered = Vec::new();
    for &qid in qids {
        let Some(q) = store.get(qid)? else { continue };
        if q.status != "open" {
            continue;
        }
        // the draft as stored, read once; Python writes dumps(loads(raw)) back
        let Some(raw) = store.draft_text(qid)? else {
            continue;
        };
        let Ok(draft) = loads(&raw) else { continue };
        if !py_truthy(&draft) {
            continue;
        }
        if !draft.is_object() {
            // Python's `d.get(...)` raises (AttributeError) on a draft that is no
            // object: the question stays open, nothing after it is answered
            return Err(rusqlite::Error::ToSqlConversionFailure(
                format!("Q{qid}'s draft is not an object: not answerable").into(),
            ));
        }
        if !lossless(&raw) {
            // a lone surrogate: Python's SQLite write raises (UnicodeEncodeError), the
            // question stays open and nothing after it is answered, delivered or marked
            return Err(rusqlite::Error::ToSqlConversionFailure(
                format!("Q{qid}'s draft holds a lone surrogate: not answerable").into(),
            ));
        }
        if store.set_answer_text(qid, &answer_text(&draft), Some(&dumps(&draft)))? {
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
        marks(pane);
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

/// Answers one open question right away with a structured `detail` (Python
/// `answer(store, qid, text, choices=…, notes=…)`, the popup's lone
/// question): its draft, then [`submit_marking`]. `None`: not open.
pub fn answer_detail(
    store: &Store,
    qid: i64,
    detail: &Value,
    deliver: Option<&mut dyn Delivery>,
    marks: &mut dyn FnMut(Option<&str>),
) -> rusqlite::Result<Option<Question>> {
    if !store.set_draft(qid, Some(detail))? {
        return Ok(None);
    }
    Ok(submit_marking(store, &[qid], deliver, marks)?
        .into_iter()
        .next())
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
    close_marking(store, qid, status, note, &mut |pane| markers.update(pane))
}

/// [`close`], the pane's marks refreshed by `marks`.
pub fn close_marking(
    store: &Store,
    qid: i64,
    status: &str,
    note: Option<&str>,
    marks: &mut dyn FnMut(Option<&str>),
) -> rusqlite::Result<bool> {
    let Some(q) = store.get(qid)? else {
        return Ok(false);
    };
    if !store.set_status(qid, status, note)? {
        return Ok(false);
    }
    marks(q.pane_id.as_deref());
    Ok(true)
}

#[cfg(test)]
mod tests;

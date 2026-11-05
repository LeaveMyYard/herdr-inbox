//! A store row as the popup model reads it: [`crate::store::Question`] and
//! [`crate::store::alerts::Alert`] into [`inbox_core::popup`]'s types, and a
//! [`Detail`] into the JSON a draft is stored as (Python `make_detail`'s
//! dict).

use inbox_core::popup::{Alert, Detail, QOption, Question};
use serde_json::{Map, Value, json};

use crate::remote::pyvalue::py_truthy;
use crate::store::Question as Row;
use crate::store::alerts::Alert as AlertRow;

/// The model's question for a stored one.
pub fn question(q: &Row) -> Question {
    Question {
        id: q.id,
        group: q.group(),
        status: q.status.clone(),
        pane_id: q.pane_id.clone(),
        options: q
            .options
            .iter()
            .flatten()
            .map(|o| QOption {
                label: o.label.clone(),
                description: o.description.clone(),
                preview: o.preview.clone(),
            })
            .collect(),
        multi_select: q.multi_select,
        draft: q.draft_json.as_ref().and_then(detail),
        delivery_error: q.delivery_error.clone(),
        delivered: q.delivered_at.is_some(),
        tab_label: q.tab_label.clone(),
        asked_by: q.asked_by.clone(),
        header: q.header.clone(),
        question: q.question.clone(),
        context: q.context.clone(),
        group_title: q.group_title.clone(),
        answer: q.answer.clone(),
        created_at: q.created_at.clone(),
    }
}

/// The model's alert for a stored one.
pub fn alert(a: &AlertRow) -> Alert {
    Alert {
        id: a.id,
        kind: a.kind.clone(),
        status: a.status.clone(),
        pane_id: a.pane_id.clone(),
        severity: a.severity.clone(),
        title: a.title.clone(),
        message: a.message.clone(),
        tab_label: a.tab_label.clone(),
        asked_by: a.asked_by.clone(),
        created_at: a.created_at.clone(),
    }
}

/// A stored draft as a [`Detail`]: none when Python's `q.draft_json` is
/// falsy (NULL, `{}`); its picks, notes and text as the popup reads them
/// (`d.get("choices") or []`, ...), anything else in it ignored.
pub fn detail(v: &Value) -> Option<Detail> {
    if !py_truthy(v) {
        return None;
    }
    let strings = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let notes = v
        .get("notes")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, n)| Some((k.clone(), n.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some(Detail {
        choices: strings("choices"),
        notes,
        text: v.get("text").and_then(Value::as_str).map(str::to_string),
    })
}

/// A [`Detail`] as it is stored (Python `make_detail`'s dict:
/// `{"choices", "notes", "text"}`, the notes in the order written).
pub fn detail_json(d: &Detail) -> Value {
    let notes: Map<String, Value> = d
        .notes
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    json!({"choices": d.choices, "notes": notes, "text": d.text})
}

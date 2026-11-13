//! A `group` row: one `ask_operator` call's open questions.

use serde_json::{Value, json};

use super::asker;
use super::text::{DESCRIPTION, LONG, PREVIEW, Text};
use super::token::token;
use crate::store::Store;
use crate::store::question::{QOption, Question};
use crate::tui::snapshot::convert::{detail, detail_json};

/// What the row takes.
pub const ACTIONS: [&str; 3] = ["answer", "draft", "dismiss"];

/// The row of group `group`, whose open questions are `open`, by id and never
/// empty, with its token. Text comes out as `text` says, but the token is
/// always of the cut text, so `app show`'s row takes the same verbs.
pub fn entry(store: &Store, group: i64, open: &[Question], text: Text) -> rusqlite::Result<Value> {
    let mut row = bare(store, group, open, text)?;
    let published = match text {
        Text::Cut => token(&row),
        Text::Whole => token(&bare(store, group, open, Text::Cut)?),
    };
    row["actions"] = json!(ACTIONS);
    row["token"] = json!(published);
    Ok(row)
}

fn bare(store: &Store, group: i64, open: &[Question], text: Text) -> rusqlite::Result<Value> {
    let first = open.first().ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let questions: Vec<Value> = open.iter().map(|q| question(q, text)).collect();
    Ok(json!({
        "key": format!("group:{group}"),
        "kind": "group",
        "group": group,
        "title": first.group_title,
        "created_at": first.created_at,
        "asker": asker::of_question(store, first)?,
        "questions": questions,
        // the call's, recorded on each question it filed; out of the token (v1's list is frozen)
        "tool_use_id": store.tool_use_id(first.id)?,
    }))
}

/// A `Question` of the feed.
fn question(q: &Question, text: Text) -> Value {
    let mut cut = false;
    let options = q.options.as_deref().unwrap_or_default();
    let options: Vec<Value> = options.iter().map(|o| option(o, text, &mut cut)).collect();
    let body = text.apply(&q.question, LONG, &mut cut);
    let context = text.apply_opt(q.context.as_deref(), LONG, &mut cut);
    json!({
        "id": q.id,
        "header": q.header,
        "question": body,
        "options": options,
        "multi_select": q.multi_select,
        "context": context,
        "asked_by": q.asked_by,
        "draft": draft(q.draft_json.as_ref()),
        "cut": cut,
    })
}

/// An `Option` of the feed: its label whole, as an answer depends on it.
pub fn option(o: &QOption, text: Text, cut: &mut bool) -> Value {
    json!({
        "label": o.label,
        "description": text.apply_opt(o.description.as_deref(), DESCRIPTION, cut),
        "preview": text.apply_opt(o.preview.as_deref(), PREVIEW, cut),
    })
}

/// A stored draft as a `Detail`, as the popup reads it. A draft the popup
/// would take for none (`{}`) is null.
fn draft(stored: Option<&Value>) -> Value {
    match stored.and_then(detail) {
        Some(d) => detail_json(&d),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests;

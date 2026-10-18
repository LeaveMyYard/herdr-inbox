//! The prompt-carrying texts of one transcript record (Python
//! `transcript._records`, `_record_texts`).
//!
//! Claude Code writes a typed prompt as a `user` record (string content, or
//! text blocks) when idle, as a `queue-operation` `enqueue` and an
//! `attachment` `queued_command` when it queues it mid-turn, a hook's
//! `additionalContext` as an `attachment` `hook_additional_context`, and an
//! asyncRewake wake as a `<task-notification>` inside a `user` record.

use herdr_agent_kit::agent_session::claude::wire::Line;
use herdr_agent_kit::internal::inbox_contract::delivery::rewake_texts;
use serde_json::Value;

/// One record's texts and its own timestamp, when that is a string.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Texts {
    /// The texts, in Python's order.
    pub texts: Vec<String>,
    /// `timestamp`, when it is a string.
    pub ts: Option<String>,
}

/// The texts of the record on `line`; none for a line that is not a JSON
/// object.
pub fn record_texts(line: &str) -> Texts {
    let Some(record) = Line::parse(line.as_bytes()) else {
        return Texts::default();
    };
    let ts = record
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut texts = Vec::new();
    match record.kind() {
        Some("user") => user_texts(&record, &mut texts),
        Some("queue-operation")
            if record.get("operation").and_then(Value::as_str) == Some("enqueue") =>
        {
            if let Some(s) = record.get("content").and_then(Value::as_str) {
                texts.push(s.to_owned());
            }
        }
        Some("attachment") => attachment_texts(record.get("attachment"), &mut texts),
        _ => {}
    }
    Texts { texts, ts }
}

fn user_texts(record: &Line, out: &mut Vec<String>) {
    let content = record.message().and_then(|m| m.get("content"));
    let text = match content {
        Some(Value::String(s)) => s.clone(),
        // a list of blocks: only its text blocks are the prompt
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                let b = b.as_object()?;
                (b.get("type").and_then(Value::as_str) == Some("text")).then_some(())?;
                b.get("text").and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return,
    };
    if text.is_empty() {
        return;
    }
    let rewake: Vec<String> = if text.contains("<task-notification>") {
        rewake_texts(&text).into_iter().map(str::to_owned).collect()
    } else {
        Vec::new()
    };
    out.push(text);
    out.extend(rewake);
}

fn attachment_texts(attachment: Option<&Value>, out: &mut Vec<String>) {
    let Some(a) = attachment.and_then(Value::as_object) else {
        return;
    };
    match a.get("type").and_then(Value::as_str) {
        Some("queued_command") => {
            if let Some(p) = a.get("prompt").and_then(Value::as_str) {
                out.push(p.to_owned());
            }
        }
        Some("hook_additional_context") => match a.get("content") {
            Some(Value::Array(items)) => {
                out.extend(items.iter().filter_map(Value::as_str).map(str::to_owned))
            }
            Some(Value::String(s)) => out.push(s.clone()),
            _ => {}
        },
        _ => {}
    }
}

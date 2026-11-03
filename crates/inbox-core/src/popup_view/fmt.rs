//! The words the popup builds from a question (Python `service.first_line`,
//! `group_name`, `format_answer`, `cli._age`, `tui.tab_name`,
//! `tui.pending_text`).

use crate::popup::{Detail, Question};

/// The `delivery_error` prefix of an answer waiting to be sent (Python
/// `store.PENDING`).
pub(crate) const PENDING: &str = "pending: ";
/// Python `service.BUSY_GRACE`: a busy agent gets its answer typed anyway
/// this long after it was answered.
const BUSY_GRACE: f64 = 20.0;
const BUSY_WAIT_IDLE: &str =
    "busy: typed while the agent was busy but never seen; waiting for it to be idle";
const PANEL: &str = "blocked: panel: the agent's background-task panel has focus";

/// Python `service.first_line`: the first line of the stripped text, cut
/// to `width` characters with an ellipsis.
pub(crate) fn first_line(text: &str, width: usize) -> String {
    let stripped = text.trim_matches(super::text::py_space);
    let line = stripped.split(is_line_break).next().unwrap_or("");
    if line.chars().count() <= width {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Python's `str.splitlines` boundaries.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{b}'
            | '\u{c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// Python `service.group_name`: the group's title, else the first
/// question's header, else its first line.
pub(crate) fn group_name(qs: &[&Question]) -> String {
    let Some(first) = qs.first() else {
        return String::new();
    };
    nonempty(&first.group_title)
        .or(nonempty(&first.header))
        .map_or_else(|| first_line(&first.question, 80), str::to_string)
}

/// Python `tui.tab_name`: the asking tab's label (the IO layer passes the
/// live one in the snapshot), else its pane.
pub(crate) fn tab_name(q: Option<&Question>) -> String {
    match q {
        None => "-".to_string(),
        Some(q) => nonempty(&q.tab_label)
            .or(nonempty(&q.pane_id))
            .unwrap_or("no pane")
            .to_string(),
    }
}

/// Python `service.format_answer` of a draft: `Postgres (note: …), text`.
pub(crate) fn answer_line(d: &Detail) -> String {
    let mut parts: Vec<String> = d
        .choices
        .iter()
        .map(|c| {
            let note = d
                .notes
                .iter()
                .find(|(k, _)| k == c)
                .map(|(_, v)| v.as_str());
            match note.filter(|n| !n.is_empty()) {
                Some(n) => format!("{} (note: {n})", quote(c)),
                None => quote(c),
            }
        })
        .collect();
    if let Some(t) = d.text.as_deref() {
        let t = t.trim_matches(super::text::py_space);
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }
    parts.join(", ")
}

/// Python `service._quote`: a label holding a separator, quoted.
fn quote(label: &str) -> String {
    if label.chars().any(|c| ",\"()·".contains(c)) {
        format!("\"{}\"", label.replace('"', "\\\""))
    } else {
        label.to_string()
    }
}

/// Python `cli._age`: `3d`, `5h`, `12m`, `40s` from `created` (ISO 8601)
/// to `now` (Unix seconds); empty if `created` does not parse.
pub(crate) fn age(created: &str, now: i64) -> String {
    let Some(then) = super::iso::unix_seconds(created) else {
        return String::new();
    };
    let secs = now - then;
    for (unit, size) in [("d", 86400), ("h", 3600), ("m", 60)] {
        if secs >= size {
            return format!("{}{unit}", secs / size);
        }
    }
    format!("{}s", secs.max(0))
}

/// The reason an answer waits (its `delivery_error` after `pending: `).
pub(crate) fn pending_reason(q: &Question) -> Option<&str> {
    q.delivery_error.as_deref()?.strip_prefix(PENDING)
}

/// Python `tui.pending_text`: why a pending answer waits, and when it goes;
/// `busy_for` is how long it has waited, in seconds.
pub(crate) fn pending_text(reason: &str, busy_for: f64) -> String {
    if reason == BUSY_WAIT_IDLE {
        return "⚠ not delivered — agent busy; will retry when it is idle".to_string();
    }
    if reason.starts_with("requeued:") {
        return "⚠ not delivered earlier — will resend when the agent is idle".to_string();
    }
    if reason == PANEL {
        return "waiting: the agent's task panel has focus (Esc there to close it)".to_string();
    }
    if reason.starts_with("busy:") {
        let left = (BUSY_GRACE - busy_for).ceil() as i64;
        if left > 0 {
            return format!("agent busy — will send in {left} s");
        }
        return "agent busy — sending at the next retry (every 10 s)".to_string();
    }
    if reason.starts_with("blocked:") {
        return "waiting: agent is at a prompt/dialog".to_string();
    }
    if reason == "waiting: the operator is in that pane" {
        return "waiting for you to leave that pane (it is focused)".to_string();
    }
    reason.to_string()
}

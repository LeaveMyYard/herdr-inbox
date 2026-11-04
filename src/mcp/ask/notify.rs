//! The herdr notification a new call raises (Python `service.notification_body`,
//! `first_line`).

use crate::store::Question;
use crate::text::py_isspace;

/// Python's `str.splitlines` boundaries.
fn line_break(c: char) -> bool {
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

/// The first line of `text`, stripped, cut to `width` characters with `…`
/// (Python `service.first_line`).
pub fn first_line(text: &str, width: usize) -> String {
    let line = text
        .trim_matches(py_isspace)
        .split(line_break)
        .next()
        .unwrap_or("");
    if line.chars().count() <= width {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The group's title; for an untitled call its first question, with its
/// header if any (a header alone, e.g. "DB choice", says too little in a toast).
pub fn notification_body(qs: &[Question]) -> String {
    let Some(first) = qs.first() else {
        return String::new();
    };
    if let Some(title) = first.group_title.as_deref().filter(|t| !t.is_empty()) {
        return title.to_owned();
    }
    let text = first_line(&first.question, 200);
    match first.header.as_deref().filter(|h| !h.is_empty()) {
        Some(h) => format!("{h}: {text}"),
        None => text,
    }
}

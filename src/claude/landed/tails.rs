//! Where in one text a delivery may start (Python `transcript._tails`).
//!
//! A delivery typed onto unsent text in the input box lands AFTER that text,
//! in one `user` record (live 2026-10-16, Q124/Q125). So the record's start
//! counts, and so does every later `[inbox] `, but there only with full
//! evidence (the caller matches the exact expected strings).

use crate::text::py_isspace;

/// `text` without Python's `_PASTE_WRAPPER`
/// (`^\s*(?:<pasted_content[^>]*>\s*)?`): Claude Code records a long prompt
/// wrapped as `\n\n<pasted_content id="…">\n[inbox] …`.
pub fn strip_paste_open(text: &str) -> &str {
    let rest = text.trim_start_matches(py_isspace);
    if let Some(tag) = rest.strip_prefix("<pasted_content")
        && let Some(end) = tag.find('>')
    {
        return tag[end + 1..].trim_start_matches(py_isspace);
    }
    rest
}

/// The text from its start (wrapper stripped), then from every later
/// `[inbox] ` in it.
pub fn tails(text: &str) -> Vec<&str> {
    let text = strip_paste_open(text);
    let mut out = vec![text];
    // Python's find from index 1: after the first character
    let mut from = text.chars().next().map_or(0, char::len_utf8);
    while let Some(at) = text.get(from..).and_then(|rest| rest.find("[inbox] ")) {
        let i = from + at;
        out.push(&text[i..]);
        from = i + 1;
    }
    out
}

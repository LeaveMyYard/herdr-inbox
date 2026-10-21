//! One field of a remote's export, cleaned as Python's `remote._text` does,
//! and the hex keys (`_HEX`).

use serde_json::Value;

/// Python `_HEX.fullmatch`: 8 to 64 lowercase hex digits.
pub fn is_hex_key(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn is_ctrl(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{200e}' | '\u{200f}')
        || ('\u{202a}'..='\u{202e}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
}

/// A field that should be text is not (Python's `ValueError("not a string")`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotAString;

/// Python `_text`: `None` stays `None`; a non-string is an error; line and
/// paragraph separators become spaces, controls (and bidi controls) go, and
/// at most `cap` characters are kept.
pub fn text(value: Option<&Value>, cap: usize) -> Result<Option<String>, NotAString> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(
            s.chars()
                .map(|c| {
                    if c == '\u{2028}' || c == '\u{2029}' {
                        ' '
                    } else {
                        c
                    }
                })
                .filter(|&c| !is_ctrl(c))
                .take(cap)
                .collect(),
        )),
        Some(_) => Err(NotAString),
    }
}

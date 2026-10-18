//! The hook's stdout (Python `json.dumps` with its defaults: `", "` and
//! `": "` separators, every non-ASCII character escaped), byte for byte.

/// `{"hookSpecificOutput": {"hookEventName": …, "additionalContext": …}}`.
pub fn hook_output(event_name: &str, context: &str) -> String {
    format!(
        "{{\"hookSpecificOutput\": {{\"hookEventName\": {}, \"additionalContext\": {}}}}}",
        json_string(event_name),
        json_string(context)
    )
}

/// A JSON string as Python's `json.dumps` writes it (`ensure_ascii=True`).
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;

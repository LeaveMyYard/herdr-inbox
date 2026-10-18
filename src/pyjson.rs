//! JSON as Python's `json.dumps` writes it with its defaults: `", "` and `": "`
//! separators, keys in insertion order, every non-ASCII character escaped
//! (`ensure_ascii`). The inbox stores JSON that both Python and Rust read and
//! write (`options_json`, `answer_json`, `draft_json`), so a value written here
//! is byte-equal to Python's.
//!
//! Floats are written as serde_json prints them; the inbox stores none.
//! [`loads`] reads what Python's `json.loads` reads, lone surrogates included.

use serde_json::Value;

/// `json.dumps(value)`.
pub fn dumps(value: &Value) -> String {
    let mut out = String::new();
    write(&mut out, value);
    out
}

fn write(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => out.push_str(&string(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&string(k));
                out.push_str(": ");
                write(out, v);
            }
            out.push('}');
        }
    }
}

/// `json.loads(text)` for the JSON the inbox stores. Python accepts an escaped
/// lone surrogate (`"\\ud83d"`) and keeps it in the string; a Rust string
/// cannot hold one, so it reads as U+FFFD. The value is therefore lossy: never
/// write it back where the stored text must survive (dump it and you get
/// `"\\ufffd"`).
///
/// Python also reads what this does not: `NaN`, `Infinity`, numbers beyond f64
/// such as `1e400`, and nesting deeper than 128. Here such a row fails the
/// read where Python's succeeds (a whole list query errors, and the hook falls
/// back to the worker's typing). Neither the MCP schema nor any Python writer
/// produces those values.
pub fn loads(text: &str) -> serde_json::Result<Value> {
    serde_json::from_str(text).or_else(|e| match lone_surrogates_replaced(text) {
        Some(fixed) => serde_json::from_str(&fixed),
        None => Err(e),
    })
}

/// `text` with every escaped lone surrogate replaced by `\\ufffd`; `None` if
/// there is none.
fn lone_surrogates_replaced(text: &str) -> Option<String> {
    let unit = |s: &str| -> Option<u16> {
        let hex = s.strip_prefix("\\u")?.get(..4)?;
        u16::from_str_radix(hex, 16).ok()
    };
    let (mut out, mut rest, mut changed) = (String::with_capacity(text.len()), text, false);
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(u) = unit(rest) else {
            let len = rest[1..].chars().next().map_or(1, |c| 1 + c.len_utf8());
            out.push_str(&rest[..len]); // an escape such as \\ or \": copied whole
            rest = &rest[len..];
            continue;
        };
        let low_follows = (0xD800..0xDC00).contains(&u)
            && unit(&rest[6..]).is_some_and(|l| (0xDC00..0xE000).contains(&l));
        if low_follows {
            out.push_str(&rest[..12]);
            rest = &rest[12..];
        } else if (0xD800..0xE000).contains(&u) {
            out.push_str("\\ufffd");
            rest = &rest[6..];
            changed = true;
        } else {
            out.push_str(&rest[..6]);
            rest = &rest[6..];
        }
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// A JSON string as `json.dumps` writes it (`ensure_ascii=True`).
pub fn string(s: &str) -> String {
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

//! Python's `repr()` of a string, as argparse quotes a rejected value
//! (`invalid question id value: 'x'`).

use crate::store::norm::printable;

/// `repr(s)`: single quotes unless `s` holds a `'` and no `"`; backslash,
/// the quote, `\t`, `\n`, `\r` escaped, and every other non-printable
/// character as `\xNN`, `\uNNNN` or `\UNNNNNNNN`.
pub fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if printable(c) => out.push(c),
            c if u32::from(c) < 0x100 => out.push_str(&format!("\\x{:02x}", u32::from(c))),
            c if u32::from(c) < 0x10000 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push_str(&format!("\\U{:08x}", u32::from(c))),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests;

//! The text of herdr-radar's sidebar block, walked as Python `radar.py`
//! walks it: strings and comments blanked ([`neutral`]), the agent rows'
//! arrays found by their brackets, and our `$inbox_q` cell added after the
//! title cell of a row (or taken out). **Pure.**

use std::sync::LazyLock;

use regex::Regex;

use super::{FALLBACK_FG, ROW_TOKEN_LIMIT, TOKEN};
use crate::text::py_isspace;

static OUR_CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#",\s*\{\s*token\s*=\s*"\$inbox_q"[^{}]*\}"#).unwrap());
static BLOCKED_FG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"token\s*=\s*"\$title_blocked"[^{}]*?fg\s*=\s*"(#[0-9a-fA-F]{3,8})""#).unwrap()
});
static UNKNOWN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"token\s*=\s*"\$title_unknown""#).unwrap());
static ANY_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"token\s*=\s*"\$title_\w+""#).unwrap());
static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\[([A-Za-z0-9_.\-]+)\][ \t]*\r?$|[ \t]*([A-Za-z0-9_\-]+)[ \t]*=[ \t]*\[)")
        .unwrap()
});

/// The tables whose arrays are agent rows.
const AGENT_TABLES: [&str; 2] = ["ui.sidebar.agents", "ui.sidebar.agents.rows_by_agent"];

/// `text` with string contents and comments blanked, byte for byte the same
/// length (a character becomes as many spaces as it has bytes), so brackets
/// in a `contains = "["` rule or a comment are not mistaken for structure.
pub fn neutral(text: &str) -> String {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<u8> = text.as_bytes().to_vec();
    let blank = |out: &mut Vec<u8>, (at, c): (usize, char)| {
        out[at..at + c.len_utf8()].fill(b' ');
    };
    let (mut i, n) = (0, chars.len());
    while i < n {
        let ch = chars[i].1;
        if ch == '#' {
            while i < n && chars[i].1 != '\n' {
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            let quote = ch;
            i += 1;
            while i < n && chars[i].1 != quote && chars[i].1 != '\n' {
                if quote == '"' && chars[i].1 == '\\' {
                    blank(&mut out, chars[i]);
                    i += 1;
                    if i >= n {
                        break;
                    }
                }
                blank(&mut out, chars[i]);
                i += 1;
            }
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The `]` closing the `[` at `open_at`, if any.
fn close(neutral: &str, open_at: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (at, b) in neutral.bytes().enumerate().skip(open_at) {
        match b {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// The `[...]` elements of the array from `open_at` to `close_at`.
fn top_rows(neutral: &str, open_at: usize, close_at: usize) -> Vec<(usize, usize)> {
    let (mut rows, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (at, b) in neutral.bytes().enumerate().take(close_at).skip(open_at + 1) {
        match b {
            b'[' => {
                if depth == 0 {
                    start = at;
                }
                depth += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    rows.push((start, at + 1));
                }
            }
            _ => {}
        }
    }
    rows
}

/// The cells of a row `[a, b, ...]`: its top-level elements, whatever their
/// form (inline tables, bare `"$x"` tokens, bare built-ins); a cell's own
/// `rules = [...]` does not count. A trailing comma counts one too many,
/// which only errs on the safe side.
pub fn tokens_in(row: &str) -> usize {
    let (mut depth, mut cells, mut seen) = (0i32, 0, false);
    for ch in neutral(row).chars() {
        match ch {
            '[' | '{' => {
                depth += 1;
                if depth == 2 {
                    seen = true;
                }
            }
            ']' | '}' => depth -= 1,
            ',' if depth == 1 => cells += 1,
            c if depth == 1 && !py_isspace(c) => seen = true,
            _ => {}
        }
    }
    if seen { cells + 1 } else { 0 }
}

/// Our cell, in the red of the row's own blocked title.
fn cell(row: &str) -> String {
    let fg = BLOCKED_FG
        .captures(row)
        .and_then(|c| c.get(1))
        .map_or(FALLBACK_FG, |m| m.as_str());
    format!("{{ token = \"{TOKEN}\", fg = \"{fg}\", bold = true, dim = false }}")
}

/// `row` with our cell right after its title cell (the `$title_unknown`
/// one, else its last `$title_*`), unless it is no title row, has it
/// already, or would pass herdr's 16-cell limit; or with ours removed.
pub fn edit_row(row: &str, remove: bool) -> String {
    if remove {
        return OUR_CELL.replace_all(row, "").into_owned();
    }
    if !row.contains("\"$title_")
        || row.contains(&format!("\"{TOKEN}\""))
        || tokens_in(row) + 1 > ROW_TOKEN_LIMIT
    {
        return row.to_string();
    }
    let anchor = UNKNOWN
        .find(row)
        .or_else(|| ANY_TITLE.find_iter(row).last());
    let Some(anchor) = anchor else {
        return row.to_string(); // Python raises here (no `token = "$title_…"` cell)
    };
    let neutral = neutral(row);
    let mut depth = 0;
    for (at, b) in neutral.bytes().enumerate().skip(anchor.end()) {
        match b {
            b'{' => depth += 1,
            b'}' if depth == 0 => {
                return format!("{}, {}{}", &row[..=at], cell(row), &row[at + 1..]);
            }
            b'}' => depth -= 1,
            _ => {}
        }
    }
    row.to_string()
}

/// `block` with our cell added to (or removed from) every agent row: `rows`
/// of `[ui.sidebar.agents]` and every entry of
/// `[ui.sidebar.agents.rows_by_agent]`. An array that never closes leaves
/// the block as it is.
pub fn edit_block(block: &str, remove: bool) -> String {
    let neutral = neutral(block);
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut table: Option<String> = None;
    for line in LINE.captures_iter(&neutral) {
        if let Some(name) = line.get(1) {
            table = Some(name.as_str().trim().to_string());
            continue;
        }
        let key = line.get(2).map_or("", |m| m.as_str());
        let Some(t) = table.as_deref().filter(|t| AGENT_TABLES.contains(t)) else {
            continue;
        };
        if t == "ui.sidebar.agents" && key != "rows" {
            continue;
        }
        let open_at = line.get(0).map_or(0, |m| m.end()) - 1;
        let Some(close_at) = close(&neutral, open_at) else {
            return block.to_string();
        };
        for (start, end) in top_rows(&neutral, open_at, close_at) {
            let new = edit_row(&block[start..end], remove);
            if new != block[start..end] {
                edits.push((start, end, new));
            }
        }
    }
    let mut out = block.to_string();
    for (start, end, new) in edits.into_iter().rev() {
        out.replace_range(start..end, &new);
    }
    out
}

#[cfg(test)]
mod tests;

//! The edit of herdr's whole `config.toml` (Python `radar.edited`,
//! `unsupported`): only inside radar's markers, only when the file parses
//! before and after, never on TOML the walker does not handle.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::rows::edit_block;
use super::{END, START};

static ARRAY_OF_TABLES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\[\[").unwrap());

/// Radar's block uses TOML this editor does not walk (`[[array.of.tables]]`,
/// multi-line strings): refuse rather than guess.
pub fn unsupported(text: &str) -> bool {
    let start = text.find(START).unwrap_or(0);
    let end = text.find(END).unwrap_or(text.len()).max(start);
    let block = &text[start..end];
    block.contains("\"\"\"") || block.contains("'''") || ARRAY_OF_TABLES.is_match(block)
}

/// The block's markers, once each and in order: `(start, end)` of the
/// block (the end marker excluded).
pub fn block_span(text: &str) -> Option<(usize, usize)> {
    let start = text.find(START)?;
    let end = text[start + 1..].find(END)? + start + 1;
    (text.matches(START).count() == 1 && text.matches(END).count() == 1).then_some((start, end))
}

/// The new file text, or `None` when nothing should change: no block, a file
/// that does not parse (`parses` is TOML 1.0, as herdr's `tomllib` reads it),
/// unsupported TOML in the block, no edit to make, or an edit that would
/// not parse (`warn`ed about, naming `shown`, as Python logs it).
pub fn edited(
    text: &str,
    remove: bool,
    parses: &dyn Fn(&str) -> bool,
    shown: &Path,
    warn: &mut dyn FnMut(&str),
) -> Option<String> {
    let (start, end) = block_span(text)?;
    if !parses(text) || unsupported(text) {
        return None;
    }
    let new = format!(
        "{}{}{}",
        &text[..start],
        edit_block(&text[start..end], remove),
        &text[end..]
    );
    if new == text {
        return None;
    }
    if !parses(&new) {
        warn(&format!(
            "radar sidebar: edit would break {}; left alone",
            shown.display()
        ));
        return None;
    }
    Some(new)
}

#[cfg(test)]
mod tests;

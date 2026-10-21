//! The text the store derives from what an agent wrote (Python `store.normalize`,
//! `store.clean_title`).
//!
//! **Unicode version.** Both follow Unicode 16.0, Python 3.14's: general
//! categories from `unicode-general-category` 1.1.0, and a character
//! unassigned there is never lowercased (Rust's std case tables are newer). The
//! plugin's Python is whatever uv picks (`>=3.11`: 3.11 is Unicode 14.0, 3.12
//! 15.0, 3.13 15.1), and they differ from each other and from this on the
//! characters added since (a few thousand). That is harmless by use:
//! - `norm` is only compared for repeats: `service.ask` (the MCP tool) looks
//!   for an open or recently answered question with the same `norm`
//!   ([`crate::store::Store::find_duplicate`]). A mismatch means a repeated
//!   question is filed again instead of merged: one more card for the operator,
//!   nothing lost or misdelivered. Python releases on different Pythons already
//!   behave this way.
//! - `clean_title` is a group title to display: a mismatch shows a character
//!   where another release shows a space.
//!
//! No hook calls either, so the Rust hooks and the Python MCP coexisting change
//! nothing here; only the MCP server switching mid-session can (P5-F).

use unicode_general_category::{GeneralCategory as G, get_general_category};

use crate::text::{py_isspace, squash};

/// The longest group title, in characters (Python `MAX_TITLE`).
pub const MAX_TITLE: usize = 60;

/// Python's `re` `\w` for one character: a letter, a number, or `_`.
fn is_word(c: char) -> bool {
    c == '_'
        || matches!(
            get_general_category(c),
            G::UppercaseLetter
                | G::LowercaseLetter
                | G::TitlecaseLetter
                | G::ModifierLetter
                | G::OtherLetter
                | G::DecimalNumber
                | G::LetterNumber
                | G::OtherNumber
        )
}

/// Python's `str.isprintable` for one character.
pub fn printable(c: char) -> bool {
    is_printable(c)
}

fn is_printable(c: char) -> bool {
    c == ' '
        || !matches!(
            get_general_category(c),
            G::Control
                | G::Format
                | G::Surrogate
                | G::PrivateUse
                | G::Unassigned
                | G::LineSeparator
                | G::ParagraphSeparator
                | G::SpaceSeparator
        )
}

/// Lowercase, punctuation as spaces, whitespace collapsed:
/// `" ".join(re.sub(r"[^\w\s]", " ", text.lower()).split())`.
pub fn normalize(text: &str) -> String {
    // unassigned in Unicode 16.0: neither a word character nor a space, so a
    // space; done before lowercasing, which would use std's newer tables
    let assigned: String = text
        .chars()
        .map(|c| {
            if get_general_category(c) == G::Unassigned {
                ' '
            } else {
                c
            }
        })
        .collect();
    let lowered: String = assigned
        .to_lowercase()
        .chars()
        .map(|c| if is_word(c) || py_isspace(c) { c } else { ' ' })
        .collect();
    squash(&lowered)
}

/// One line, no control characters, at most [`MAX_TITLE`] characters (a longer
/// one keeps `MAX_TITLE - 1` and `…`); `None` when nothing is left.
pub fn clean_title(title: Option<&str>) -> Option<String> {
    let title = title.filter(|t| !t.is_empty())?;
    let printable: String = title
        .chars()
        .map(|c| if is_printable(c) { c } else { ' ' })
        .collect();
    let text = squash(&printable);
    let text = if text.chars().count() > MAX_TITLE {
        let mut cut: String = text.chars().take(MAX_TITLE - 1).collect();
        cut.push('…');
        cut
    } else {
        text
    };
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests;

//! Python's string classes, where the inbox's text output depends on them.

use unicode_general_category::{GeneralCategory as G, get_general_category};

/// Python's `re` `\w` for one character: a letter, a number, or `_`.
pub fn is_word(c: char) -> bool {
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

/// Python's `str.isspace` for one character: Unicode `White_Space` plus the
/// four separators U+001C..U+001F, which Python also splits on.
pub fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `" ".join(text.split())`: runs of whitespace as one space, the
/// ends trimmed.
pub fn squash(text: &str) -> String {
    text.split(py_isspace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;

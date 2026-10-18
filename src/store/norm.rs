//! The text the store derives from what an agent wrote (Python `store.normalize`,
//! `store.clean_title`). `norm` is compared across releases (a repeated question
//! is the same question), so it must be Python's to the character.

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
    let lowered: String = text
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

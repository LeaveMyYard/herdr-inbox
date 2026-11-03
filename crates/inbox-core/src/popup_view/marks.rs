//! The pieces every option-like row starts with (Python `_cursor` and the
//! numbers), and the bold-when-focused label.

use super::text::{Ln, Look};
use super::theme::Theme;

/// Python `_cursor`: ❯ on the focused row, else one blank cell in the
/// background colour (its own span, as the Python popup draws it).
pub(crate) fn cursor(here: bool, theme: &Theme) -> Ln {
    if here {
        Ln::new().on(Look::fg(theme.cc.suggestion), "❯")
    } else {
        Ln::new().on(Look::fg(theme.background), " ")
    }
}

/// `[$text-muted]N.[/]`.
pub(crate) fn number(text: &str, theme: &Theme) -> Ln {
    Ln::new().on(Look::fg(theme.number), text)
}

/// `{cursor} {number} ` then the label, bold suggestion when focused.
pub(crate) fn option(here: bool, number: &str, label: &str, theme: &Theme) -> Ln {
    cursor(here, theme)
        .plain(" ")
        .push(self::number(number, theme))
        .plain(" ")
        .push(focused(here, label, theme))
}

/// `[b $inbox-suggestion]label[/]` when focused, else `[$foreground]label[/]`.
pub(crate) fn focused(here: bool, label: &str, theme: &Theme) -> Ln {
    let look = if here {
        Look::bold(theme.cc.suggestion)
    } else {
        Look::fg(theme.foreground)
    };
    Ln::new().on(look, label)
}

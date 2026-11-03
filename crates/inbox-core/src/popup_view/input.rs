//! Textual's `Input` with the focus, as the popup's note and "Type
//! something." boxes show it: a tall frame in `$border`, two cells of
//! padding, the text with a block cursor (a placeholder when empty; all of
//! it selected when the box opened on it), the window scrolled to keep the
//! cursor in it.

use super::text::{Line, Ln, Look, char_width};
use super::theme::Theme;
use crate::empty_inbox::Rgb;

/// The box's frame and padding, in cells across.
const CHROME: u16 = 2 + 4;

/// What the box holds.
pub(crate) struct Field<'a> {
    pub value: &'a str,
    /// In characters from the start.
    pub cursor: usize,
    pub placeholder: &'a str,
    /// All of the text is selected (a box opened on existing text).
    pub selected: bool,
}

/// The box's three rows, `width` cells wide, on a screen filled `outer`.
pub(crate) fn rows(field: &Field, width: u16, outer: Rgb, theme: &Theme) -> Vec<Line> {
    let inner = usize::from(width.saturating_sub(2));
    let edge = |mid: &str| {
        Ln::new()
            .on(Look::fg(outer).on_bg(theme.border), "▊")
            .on(
                Look::fg(theme.border).on_bg(theme.input),
                &mid.repeat(inner),
            )
            .on(Look::fg(theme.border).on_bg(outer), "▎")
            .done()
    };
    let content = usize::from(width.saturating_sub(CHROME));
    let middle = Ln::new()
        .on(Look::fg(outer).on_bg(theme.border), "▊")
        .on(Look::pad(theme.input), "  ")
        .push(text(field, content, theme))
        .on(Look::pad(theme.input), "  ")
        .on(Look::fg(theme.border).on_bg(outer), "▎");
    vec![edge("▔"), middle.done(), edge("▁")]
}

/// The text row, `width` cells.
fn text(field: &Field, width: usize, theme: &Theme) -> Ln {
    let cursor = Look::fg(theme.background).on_bg(theme.foreground);
    let plain = Look::fg(theme.foreground).on_bg(theme.input);
    let text = if field.selected {
        plain.on_bg(theme.selection)
    } else {
        plain
    };
    if field.value.is_empty() {
        let mut chars = field.placeholder.chars();
        let first = chars.next().map_or(" ".to_string(), String::from);
        let rest: String = chars.collect();
        let used = super::text::width(&first) + super::text::width(&rest);
        let shown = Ln::new()
            .on(cursor, &first)
            .on(Look::fg(theme.disabled).on_bg(theme.input), &rest);
        return shown.on(plain, &" ".repeat(width.saturating_sub(used)));
    }
    let mut chars: Vec<char> = field.value.chars().collect();
    let at = field.cursor.min(chars.len());
    if at == chars.len() {
        chars.push(' ');
    }
    let len = field.value.chars().count();
    let look = |i: usize| match i {
        _ if i == at => cursor,
        _ if i < len => text,
        _ => plain,
    };
    let cells: Vec<(char, Look)> = chars
        .iter()
        .enumerate()
        .map(|(i, &c)| (c, look(i)))
        .collect();
    let start = scroll(&cells, at, width);
    let mut out = Ln::new();
    let mut used = 0;
    for &(c, look) in &cells[start..] {
        let w = char_width(c);
        if used + w > width {
            break;
        }
        used += w;
        out = out.on(look, c.encode_utf8(&mut [0; 4]));
    }
    out.on(plain, &" ".repeat(width - used))
}

/// The first character shown: the least scroll that keeps the cursor's
/// cell in the window.
fn scroll(cells: &[(char, Look)], at: usize, width: usize) -> usize {
    let mut start = 0;
    let span = |s: usize| {
        cells[s..=at]
            .iter()
            .map(|(c, _)| char_width(*c))
            .sum::<usize>()
    };
    while start < at && span(start) > width {
        start += 1;
    }
    start
}

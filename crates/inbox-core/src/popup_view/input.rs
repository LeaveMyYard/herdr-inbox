//! Textual's `Input`, as the popup's note and "Type something." boxes show
//! it: a tall frame, two cells of padding, the text, the window scrolled to
//! keep the cursor in it. With the focus the frame is `$border` and the
//! text has a block cursor (a placeholder when empty; all of it selected
//! when the box opened on it); after Tab moved the focus away the box stays
//! open, blurred: `$border-blurred`, `$surface`, no cursor.

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
    /// The box has the keys (the model's `box_focused`).
    pub focused: bool,
}

/// The colours of a focused or a blurred box.
struct Skin {
    border: Rgb,
    fill: Rgb,
    disabled: Rgb,
}

impl Skin {
    fn of(focused: bool, t: &Theme) -> Skin {
        match focused {
            true => Skin {
                border: t.border,
                fill: t.input,
                disabled: t.disabled,
            },
            false => Skin {
                border: t.border_blurred,
                fill: t.input_blurred,
                disabled: t.disabled_blurred,
            },
        }
    }
}

/// The box's three rows, `width` cells wide, on a screen filled `outer`.
pub(crate) fn rows(field: &Field, width: u16, outer: Rgb, theme: &Theme) -> Vec<Line> {
    let skin = Skin::of(field.focused, theme);
    let inner = usize::from(width.saturating_sub(2));
    let left = Look::fg(outer).on_bg(skin.border);
    let right = Look::fg(skin.border).on_bg(outer);
    let edge = |mid: &str| {
        let rule = Look::fg(skin.border).on_bg(skin.fill);
        Ln::new()
            .on(left, "▊")
            .on(rule, &mid.repeat(inner))
            .on(right, "▎")
            .done()
    };
    let content = usize::from(width.saturating_sub(CHROME));
    let middle = Ln::new()
        .on(left, "▊")
        .on(Look::pad(skin.fill), "  ")
        .push(text(field, content, &skin, theme))
        .on(Look::pad(skin.fill), "  ")
        .on(right, "▎");
    vec![edge("▔"), middle.done(), edge("▁")]
}

/// The text row, `width` cells.
fn text(field: &Field, width: usize, skin: &Skin, theme: &Theme) -> Ln {
    let plain = Look::fg(theme.foreground).on_bg(skin.fill);
    let cursor = match field.focused {
        true => Look::fg(theme.background).on_bg(theme.foreground),
        false => plain,
    };
    let text = match field.selected && field.focused {
        true => plain.on_bg(theme.selection),
        false => plain,
    };
    if field.value.is_empty() {
        let mut chars = field.placeholder.chars();
        let first = chars.next().map_or(" ".to_string(), String::from);
        let rest: String = chars.collect();
        let used = super::text::width(&first) + super::text::width(&rest);
        let hint = Look::fg(skin.disabled).on_bg(skin.fill);
        let shown = Ln::new()
            .on(if field.focused { cursor } else { hint }, &first)
            .on(hint, &rest);
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

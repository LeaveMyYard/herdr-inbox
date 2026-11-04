//! Drawing styled lines into a ratatui [`Buffer`]: a span's own colour or
//! the widget's, its own fill or the cell's, then dim and reverse resolved
//! (as Textual's compositor hands them to the terminal). Only the cells a
//! line covers are written: the rest of a widget keeps the default
//! foreground, as Textual leaves it.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};

use super::text::{Line, Look, char_width};
use super::theme::dim;
use crate::empty_inbox::Rgb;

/// A cell's colours and attributes before it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ink {
    /// `None`: the terminal's default.
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
}

pub(crate) fn color(c: Option<Rgb>) -> Color {
    c.map_or(Color::Reset, |Rgb(r, g, b)| Color::Rgb(r, g, b))
}

pub(crate) fn rgb(c: Color) -> Option<Rgb> {
    match c {
        Color::Rgb(r, g, b) => Some(Rgb(r, g, b)),
        _ => None,
    }
}

impl Ink {
    pub fn style(self) -> Style {
        let mut s = Style::new().fg(color(self.fg)).bg(color(self.bg));
        if self.bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        s
    }
}

/// The cell at `(x, y)` gets `symbol` in `ink`, if `(x, y)` is inside `clip`.
pub(crate) fn put(buf: &mut Buffer, clip: Rect, x: u16, y: u16, symbol: &str, ink: Ink) {
    let inside = x >= clip.x && x < clip.right() && y >= clip.y && y < clip.bottom();
    if let Some(cell) = buf.cell_mut((x, y)).filter(|_| inside) {
        cell.reset();
        cell.set_symbol(symbol).set_style(ink.style());
    }
}

/// The fill of the cell at `(x, y)` (the screen's, a panel's).
pub(crate) fn fill_at(buf: &Buffer, x: u16, y: u16) -> Option<Rgb> {
    buf.cell((x, y)).and_then(|c| rgb(c.bg))
}

/// Draw `line` from `(x, y)` in a widget coloured `base`, within `clip`;
/// returns the width drawn.
pub(crate) fn line(buf: &mut Buffer, clip: Rect, x: u16, y: u16, line: &Line, base: Rgb) -> u16 {
    let mut at = x;
    for span in line {
        for c in span.text.chars() {
            let w = char_width(c) as u16;
            if w == 0 {
                continue;
            }
            let ink = ink(span.look, base, fill_at(buf, at, y));
            put(buf, clip, at, y, c.encode_utf8(&mut [0; 4]), ink);
            for k in 1..w {
                put(buf, clip, at + k, y, "", ink);
            }
            at = at.saturating_add(w);
        }
    }
    at - x
}

/// A look resolved over a widget colour and the cell's fill.
pub(crate) fn ink(look: Look, base: Rgb, fill: Option<Rgb>) -> Ink {
    let bg = look.bg.or(fill);
    let mut fg = look.fg.unwrap_or(base);
    if let Some(b) = bg.filter(|_| look.dim) {
        fg = dim(fg, b);
    }
    let (fg, bg) = if look.reverse {
        (bg, Some(fg))
    } else if look.bare {
        (None, bg)
    } else {
        (Some(fg), bg)
    };
    Ink {
        fg,
        bg,
        bold: look.bold,
        italic: look.italic,
    }
}

/// Every cell of `area` blank on `bg`, default foreground.
pub(crate) fn clear(buf: &mut Buffer, area: Rect, bg: Rgb) {
    let ink = Ink {
        fg: None,
        bg: Some(bg),
        bold: false,
        italic: false,
    };
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            put(buf, area, x, y, " ", ink);
        }
    }
}

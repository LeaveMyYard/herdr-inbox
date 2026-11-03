//! Styled text as the popup's Textual markup builds it: spans with an
//! optional colour, bold, italic, dim or reverse, in lines. A span without a
//! colour takes the widget's own (Textual's `color` of the `Static`).

use unicode_width::UnicodeWidthChar;

use crate::empty_inbox::Rgb;

/// A span's look; `None` colours are the widget's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Look {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    /// Textual's `dim`: the colour a third of the way to the background.
    pub dim: bool,
    /// fg and bg swapped (the chips, the header chip).
    pub reverse: bool,
    /// The terminal's default foreground, not the widget's (padding).
    pub bare: bool,
}

impl Look {
    pub fn fg(c: Rgb) -> Look {
        Look {
            fg: Some(c),
            ..Look::default()
        }
    }

    /// `[b $colour]`.
    pub fn bold(c: Rgb) -> Look {
        Look {
            bold: true,
            ..Look::fg(c)
        }
    }

    pub fn dimmed() -> Look {
        Look {
            dim: true,
            ..Look::default()
        }
    }

    /// Blank padding on `bg`.
    pub fn pad(bg: Rgb) -> Look {
        Look {
            bg: Some(bg),
            bare: true,
            ..Look::default()
        }
    }

    pub fn on_bg(self, bg: Rgb) -> Look {
        Look {
            bg: Some(bg),
            ..self
        }
    }

    pub fn with_bold(self) -> Look {
        Look { bold: true, ..self }
    }

    pub fn italic() -> Look {
        Look {
            italic: true,
            ..Look::default()
        }
    }
}

/// Text in one look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub text: String,
    pub look: Look,
}

/// One line of spans (no newlines inside).
pub(crate) type Line = Vec<Span>;

/// A builder for a line: `.plain("x").on(look, "y")`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ln(pub Line);

impl Ln {
    pub fn new() -> Ln {
        Ln(Vec::new())
    }

    /// Text in the widget's own look.
    pub fn plain(self, text: &str) -> Ln {
        self.on(Look::default(), text)
    }

    pub fn on(mut self, look: Look, text: &str) -> Ln {
        if !text.is_empty() {
            self.0.push(Span {
                text: text.to_string(),
                look,
            });
        }
        self
    }

    pub fn push(mut self, other: Ln) -> Ln {
        self.0.extend(other.0);
        self
    }

    pub fn done(self) -> Line {
        self.0
    }
}

/// Text that may hold newlines, split into lines of one look (Python passes
/// such text through `escape` into one markup span).
pub(crate) fn lines_of(text: &str, look: Look) -> Vec<Line> {
    text.split('\n')
        .map(|t| Ln::new().on(look, t).done())
        .collect()
}

/// A character's width in cells (the popup's marks are one cell wide in
/// Textual's table and here, ⏳ two).
pub(crate) fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// A string's width in cells.
pub(crate) fn width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// A line's width in cells.
pub(crate) fn line_width(line: &Line) -> usize {
    line.iter().map(|s| width(&s.text)).sum()
}

/// Python's `str.isspace` for one character (the `\s` of its regexes).
pub(crate) fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

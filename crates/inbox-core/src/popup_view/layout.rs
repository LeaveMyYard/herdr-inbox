//! Textual's vertical layout as the popup's CSS uses it: widgets stacked
//! with collapsing margins, a `1fr` widget taking what is left, a `Static`
//! as its wrapped lines, and `dock: bottom` hints.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::canvas;
use super::text::Line;
use super::wrap::wrap;
use crate::empty_inbox::Rgb;

/// `lines` as a `width`-cell `Static` shows them.
pub(crate) fn rows(lines: &[Line], width: u16) -> Vec<Line> {
    lines.iter().flat_map(|l| wrap(l, width.into())).collect()
}

/// Draw rows from the top of `area` (clipped to it) in a widget coloured `base`.
pub(crate) fn draw(buf: &mut Buffer, area: Rect, rows: &[Line], base: Rgb) {
    for (i, row) in rows.iter().enumerate().take(area.height.into()) {
        canvas::line(buf, area, area.x, area.y + i as u16, row, base);
    }
}

/// A `Static` of `lines` placed in `stack` with margins `(top, bottom)`, drawn.
pub(crate) fn text(
    buf: &mut Buffer,
    stack: &mut Stack,
    lines: &[Line],
    margin: (u16, u16),
    base: Rgb,
) {
    let rows = rows(lines, stack.area.width);
    let area = stack.place(rows.len() as u16, margin);
    draw(buf, area, &rows, base);
}

/// The one-row hint docked at the bottom of `area` (Python `.hint`:
/// `dock: bottom; height: 1`): its first wrapped line; `area` loses the row.
pub(crate) fn hint(buf: &mut Buffer, area: &mut Rect, line: &Line, base: Rgb) {
    if area.height == 0 {
        return;
    }
    area.height -= 1;
    let row = Rect::new(area.x, area.bottom(), area.width, 1);
    draw(
        buf,
        row,
        &rows(std::slice::from_ref(line), area.width)[..1],
        base,
    );
}

/// Widgets placed top to bottom in `area`.
pub(crate) struct Stack {
    pub area: Rect,
    y: u16,
    margin: u16,
}

impl Stack {
    pub fn new(area: Rect) -> Stack {
        Stack {
            area,
            y: area.y,
            margin: 0,
        }
    }

    /// The next widget, `height` rows with margins `(top, bottom)`: the gap
    /// above it is the larger of its top margin and the bottom margin before.
    pub fn place(&mut self, height: u16, (top, bottom): (u16, u16)) -> Rect {
        let y = (self.y + self.margin.max(top)).min(self.area.bottom());
        let height = height.min(self.area.bottom() - y);
        self.y = y + height;
        self.margin = bottom;
        Rect::new(self.area.x, y, self.area.width, height)
    }

    /// A `1fr` widget with margins `(top, bottom)`: every row left.
    pub fn rest(&mut self, (top, bottom): (u16, u16)) -> Rect {
        let y = (self.y + self.margin.max(top)).min(self.area.bottom());
        let height = self.area.bottom().saturating_sub(y + bottom);
        self.place(height, (top, bottom))
    }
}

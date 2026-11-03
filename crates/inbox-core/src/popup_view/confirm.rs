//! The dismiss confirmation (Python `ConfirmDismiss`, a `ModalScreen`): the
//! screen behind it faded to 40% (Textual's `$background 60%` over it, the
//! screen drawn inside the modal's own padding), and a rounded box in
//! `$panel`, framed in warning, centred on it.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::canvas::{self, Ink, rgb};
use super::layout;
use super::text::{Line, Ln, Look, line_width};
use super::theme::{Theme, blend};

/// `#confirm { max-width: 80%; padding: 1 2; border: round }`.
const PAD: (u16, u16) = (1, 2);

/// Fade `buf` behind the modal (`inner`: the screen less its padding).
pub(crate) fn fade(buf: &mut Buffer, inner: Rect, theme: &Theme) {
    let behind = buf.clone();
    let area = buf.area;
    canvas::clear(buf, area, theme.background);
    for y in inner.top()..inner.bottom() {
        for x in inner.left()..inner.right() {
            let from = behind.cell((x - inner.x + area.x, y - inner.y + area.y));
            let Some(cell) = from else { continue };
            let fade = |c| blend(c, theme.background, 0.6);
            let ink = Ink {
                fg: Some(rgb(cell.fg).map_or(theme.foreground, fade)),
                bg: rgb(cell.bg).map(fade),
                bold: cell.modifier.contains(ratatui_core::style::Modifier::BOLD),
                italic: cell
                    .modifier
                    .contains(ratatui_core::style::Modifier::ITALIC),
            };
            canvas::put(buf, inner, x, y, cell.symbol(), ink);
        }
    }
}

/// Draw the box: "Dismiss <what> without answering?", the dim `detail`,
/// then "y to dismiss · any other key to keep it".
pub(crate) fn draw(buf: &mut Buffer, inner: Rect, what: &str, detail: &str, theme: &Theme) {
    let lines: Vec<Line> = vec![
        Ln::new()
            .plain("Dismiss ")
            .on(Look::default().with_bold(), what)
            .plain(" without answering?")
            .done(),
        Ln::new().on(Look::dimmed(), detail).done(),
        Vec::new(),
        Ln::new()
            .on(Look::fg(theme.cc.warning), "y")
            .plain(" to dismiss · any other key to keep it")
            .done(),
    ];
    let chrome = 2 * PAD.1 + 2;
    let max = (u32::from(inner.width) * 80 / 100) as u16;
    let widest = lines.iter().map(line_width).max().unwrap_or(0) as u16;
    let content = widest.min(max.saturating_sub(chrome));
    let rows = layout::rows(&lines, content);
    let (w, h) = (content + chrome, rows.len() as u16 + 2 * PAD.0 + 2);
    let x = inner.x + inner.width.saturating_sub(w) / 2;
    let y = inner.y + inner.height.saturating_sub(h) / 2;
    let area = Rect::new(x, y, w, h).intersection(buf.area);
    canvas::clear(buf, area, theme.panel);
    let frame = Ink {
        fg: Some(theme.cc.warning),
        bg: Some(theme.panel),
        bold: false,
        italic: false,
    };
    for cx in area.left()..area.right() {
        let (top, bottom) = match cx {
            _ if cx == area.left() => ("╭", "╰"),
            _ if cx + 1 == area.right() => ("╮", "╯"),
            _ => ("─", "─"),
        };
        canvas::put(buf, area, cx, area.top(), top, frame);
        canvas::put(buf, area, cx, area.bottom() - 1, bottom, frame);
    }
    for cy in area.top() + 1..area.bottom() - 1 {
        canvas::put(buf, area, area.left(), cy, "│", frame);
        canvas::put(buf, area, area.right() - 1, cy, "│", frame);
    }
    let text =
        Rect::new(x + 1 + PAD.1, y + 1 + PAD.0, content, rows.len() as u16).intersection(area);
    layout::draw(buf, text, &rows, theme.foreground);
}

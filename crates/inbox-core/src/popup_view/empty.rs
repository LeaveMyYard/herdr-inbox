//! The empty popup: design 2C's tumbleweed in the desert
//! ([`crate::empty_inbox`], Q179), centred. When questions arrived while it
//! showed, its line says so instead of "No open questions" (Python
//! `render_empty`).

use ratatui_core::buffer::Buffer;

use super::canvas::{self, Ink};
use super::text::{char_width, width};
use crate::empty_inbox::{self, H, W, palette};

/// The design's text row (`centre(s, 15, …)`).
const TEXT_ROW: usize = 15;

/// Draw frame `elapsed` seconds into the scene, with `live` question
/// groups waiting for Tab/→.
pub(crate) fn draw(buf: &mut Buffer, elapsed: f64, live: usize) {
    let grid = empty_inbox::frame_at(elapsed);
    let area = buf.area;
    let x0 = area.x + area.width.saturating_sub(W as u16) / 2;
    let y0 = area.y + area.height.saturating_sub(H as u16) / 2;
    let text = match live {
        0 => None,
        1 => Some("1 new question group — press Tab/→ to view".to_string()),
        n => Some(format!("{n} new question groups — press Tab/→ to view")),
    };
    for y in 0..grid.height() {
        let mut row: Vec<_> = grid.row(y).to_vec();
        if let (Some(t), TEXT_ROW) = (&text, y) {
            for cell in &mut row[1..W - 1] {
                cell.ch = ' ';
            }
            let start = W.saturating_sub(width(t)) / 2;
            for (i, ch) in t
                .chars()
                .filter(|c| char_width(*c) == 1)
                .enumerate()
                .take(W - start - 1)
            {
                row[start + i].ch = ch;
                row[start + i].fg = palette::TEXT;
            }
        }
        for (x, cell) in row.iter().enumerate() {
            let ink = Ink {
                fg: Some(cell.fg),
                bg: Some(cell.bg),
                bold: cell.bold,
                italic: false,
            };
            let (cx, cy) = (x0 + x as u16, y0 + y as u16);
            canvas::put(buf, area, cx, cy, cell.ch.encode_utf8(&mut [0; 4]), ink);
        }
    }
}

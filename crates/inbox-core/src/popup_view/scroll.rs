//! Textual's `VerticalScroll` as the popup uses it: items stacked, the
//! window kept on the cursor's item (`scroll_visible`, the least distance),
//! and a two-cell scrollbar when the items overflow (which narrows them).
//! The scroll offset is the caller's memory between frames ([`super::ViewState`]).

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::canvas::{self, Ink};
use super::text::Line;
use super::theme::Theme;
use super::wrap::wrap;
use crate::empty_inbox::Rgb;

/// Textual's `scrollbar-size-vertical`.
const BAR: u16 = 2;

/// One child of the scroll box.
pub(crate) enum Item {
    /// A `Static`: its lines, wrapped to the box.
    Text(Vec<Line>),
    /// A fixed-size child laid out by its own code (an `Input`): `indent`
    /// cells from the left, its rows already built for a given width.
    Fixed {
        indent: u16,
        rows: Box<dyn Fn(u16) -> Vec<Line>>,
        height: u16,
    },
}

impl Item {
    fn rows(&self, width: u16) -> (u16, Vec<Line>) {
        match self {
            Item::Text(lines) => (
                0,
                lines.iter().flat_map(|l| wrap(l, width.into())).collect(),
            ),
            Item::Fixed {
                indent,
                rows,
                height,
            } => {
                let mut r = rows(width.saturating_sub(*indent));
                r.resize(usize::from(*height), Vec::new());
                (*indent, r)
            }
        }
    }
}

/// The box's look: the widget colour, and `gutter` cells of right padding
/// between the items and the scrollbar's place.
pub(crate) struct Frame<'a> {
    pub base: Rgb,
    pub gutter: u16,
    pub theme: &'a Theme,
}

/// Draw `items` into `area`, keeping the items `visible` (in order) in the
/// window; `offset` is the scroll position, read and updated.
pub(crate) fn draw(
    buf: &mut Buffer,
    area: Rect,
    items: &[Item],
    visible: &[usize],
    offset: &mut u16,
    frame: &Frame,
) {
    let (base, theme) = (frame.base, frame.theme);
    let mut width = area.width.saturating_sub(frame.gutter);
    let mut laid = layout(items, width);
    let total = laid.iter().map(|(_, rows)| rows.len() as u16).sum::<u16>();
    let bar = total > area.height && area.width > BAR;
    if bar {
        width -= BAR;
        laid = layout(items, width);
    }
    let total: u16 = laid.iter().map(|(_, rows)| rows.len() as u16).sum();
    let max = total.saturating_sub(area.height);
    *offset = (*offset).min(max);
    let tops: Vec<u16> = laid
        .iter()
        .scan(0u16, |y, (_, rows)| {
            let top = *y;
            *y += rows.len() as u16;
            Some(top)
        })
        .collect();
    for &i in visible {
        if let Some((_, rows)) = laid.get(i) {
            *offset = to_visible(*offset, area.height, tops[i], rows.len() as u16).min(max);
        }
    }
    for ((indent, rows), top) in laid.iter().zip(&tops) {
        for (k, row) in rows.iter().enumerate() {
            let y = i32::from(*top) + k as i32 - i32::from(*offset);
            if (0..i32::from(area.height)).contains(&y) {
                let clip = Rect::new(area.x, area.y, width, area.height);
                canvas::line(buf, clip, area.x + indent, area.y + y as u16, row, base);
            }
        }
    }
    if bar {
        scrollbar(
            buf,
            Rect::new(area.right() - BAR, area.y, BAR, area.height),
            total,
            *offset,
            base,
            theme,
        );
    }
}

fn layout(items: &[Item], width: u16) -> Vec<(u16, Vec<Line>)> {
    items.iter().map(|it| it.rows(width)).collect()
}

/// `Region.get_scroll_to_visible` on the y axis: the least move that shows
/// rows `top..top + height` (cropped to the window), clamped at 0.
fn to_visible(offset: u16, window: u16, top: u16, height: u16) -> u16 {
    let height = height.min(window);
    let (wt, wb) = (i32::from(offset), i32::from(offset) + i32::from(window));
    let (t, b) = (i32::from(top), i32::from(top) + i32::from(height));
    if t >= wt && b <= wb {
        return offset;
    }
    let up = t - wt;
    let down = t - (wb - i32::from(height));
    let delta = if down.abs() < up.abs() { down } else { up };
    (wt + delta).max(0) as u16
}

/// Textual's `ScrollBarRender.render_bar` (vertical): the thumb with
/// eighth-cell ends, on the track.
fn scrollbar(buf: &mut Buffer, area: Rect, total: u16, offset: u16, base: Rgb, theme: &Theme) {
    const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", " "];
    let size = f64::from(area.height);
    let (virt, window) = (f64::from(total), f64::from(area.height));
    let thumb = (window / (virt / size)).max(1.0);
    let ratio = f64::from(offset) / (virt - window);
    let position = (size - thumb) * ratio;
    let start = (position * 8.0) as usize;
    let end = start + (thumb * 8.0).ceil() as usize;
    let (start_i, start_bar) = (start / 8, start % 8);
    let (end_i, end_bar) = (end / 8, end % 8);
    let ink = |fg: Option<Rgb>, bg: Rgb| Ink {
        fg,
        bg: Some(bg),
        bold: false,
        italic: false,
    };
    for row in 0..area.height {
        let i = usize::from(row);
        let (symbol, cell) = if i == end_i && BARS[7 - end_bar] != " " {
            (BARS[7 - end_bar], ink(Some(theme.track), theme.thumb))
        } else if i == start_i && BARS[7 - start_bar] != " " {
            (BARS[7 - start_bar], ink(Some(theme.thumb), theme.track))
        } else if (start_i..end_i).contains(&i) {
            (" ", ink(Some(theme.background), theme.thumb))
        } else {
            (" ", ink(Some(base), theme.track))
        };
        for x in area.left()..area.right() {
            canvas::put(buf, area, x, area.y + row, symbol, cell);
        }
    }
}

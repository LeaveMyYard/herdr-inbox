//! Above an open group's view: its name (Python `render_group_title`) and
//! its tabs (`render_chips`): `│ tab`, then each question's chip (✔ when
//! drafted, ● when not, ✕ when it closed) and Submit, the current one
//! reversed, in a row that scrolls sideways to keep it in view.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::canvas;
use super::fmt::{group_name, tab_name};
use super::layout::{self, Stack};
use super::text::{Line, Ln, Look, line_width};
use super::theme::Theme;
use crate::popup::{Entry, Mode, Popup, Tab};

/// `#chips > Static { margin-right: 2 }`.
const GAP: u16 = 2;

/// The group's name, if the title row shows (a titled group, or several
/// questions; a lone untitled one shows its header chip instead).
pub(crate) fn title(popup: &Popup) -> Option<Line> {
    let (Mode::Group, Some(Entry::Group(g))) = (popup.mode(), popup.entry()) else {
        return None;
    };
    let qs = popup.data().group(*g);
    let first = qs.first()?;
    let titled = first.group_title.as_deref().is_some_and(|t| !t.is_empty());
    if !titled && qs.len() <= 1 {
        return None;
    }
    let live: Vec<_> = qs.iter().copied().filter(|q| q.status == "open").collect();
    let name = group_name(if live.is_empty() { &qs } else { &live });
    Some(Ln::new().plain(&name).done())
}

/// Draw the title row (bold, `margin-bottom: 1`) into `stack`.
pub(crate) fn draw_title(buf: &mut Buffer, stack: &mut Stack, line: &Line, theme: &Theme) {
    let bold: Line = line
        .iter()
        .cloned()
        .map(|mut s| {
            s.look.bold = true;
            s
        })
        .collect();
    layout::text(buf, stack, &[bold], (0, 1), theme.foreground);
}

/// The tab row's chips, if it shows (more than one tab): each with the
/// colour of its widget.
pub(crate) fn chips(popup: &Popup, theme: &Theme) -> Option<Vec<(Line, bool)>> {
    let tabs = if popup.mode() == Mode::Group {
        popup.all_tabs()
    } else {
        Vec::new()
    };
    if tabs.len() <= 1 {
        return None;
    }
    let data = popup.data();
    let first = popup.tabs().first().and_then(|t| match t {
        Tab::Question(id) => data.question(*id),
        Tab::Submit => None,
    });
    let sep = Ln::new().on(Look::fg(theme.muted), &format!("│ {}", tab_name(first)));
    let mut out = vec![(sep.done(), false)];
    for tab in tabs {
        let here = popup.current() == Some(tab);
        let (mark, label, warn) = match tab {
            Tab::Submit => (Ln::new(), "Submit".to_string(), false),
            Tab::Question(id) if popup.gone().contains(&tab) => {
                (Ln::new().plain("✕\u{a0}"), format!("Q{id}"), true)
            }
            Tab::Question(id) => {
                let q = data.question(id);
                let mark = match q.is_some_and(|q| q.draft.is_some()) {
                    true => Ln::new().on(Look::fg(theme.cc.success), "✔\u{a0}"),
                    false => Ln::new().plain("●\u{a0}"),
                };
                let label = q
                    .and_then(|q| q.header.clone().filter(|h| !h.is_empty()))
                    .unwrap_or_else(|| format!("Q{id}"));
                (mark, label, false)
            }
        };
        let chip = if here {
            let rev = Look {
                reverse: true,
                bold: true,
                ..Look::default()
            };
            mark.on(rev, &format!(" {label} "))
        } else {
            mark.plain(&label)
        };
        out.push((chip.done(), warn));
    }
    Some(out)
}

/// Draw the tab row (`height: 1; margin-bottom: 1`) into `stack`, scrolled
/// from `scroll_x` to keep the current chip in view; returns the new scroll.
pub(crate) fn draw_chips(
    buf: &mut Buffer,
    stack: &mut Stack,
    chips: &[(Line, bool)],
    current: Option<usize>,
    scroll_x: u16,
    theme: &Theme,
) -> u16 {
    let area = stack.place(1, (0, 1));
    let spans: Vec<(u16, u16)> = chips
        .iter()
        .scan(0u16, |x, (line, _)| {
            let at = *x;
            let w = line_width(line) as u16;
            *x += w + GAP;
            Some((at, w + GAP))
        })
        .collect();
    let total = spans.last().map_or(0, |(x, w)| x + w);
    let max = total.saturating_sub(area.width);
    let mut scroll = scroll_x.min(max);
    if let Some((x, w)) = current.and_then(|i| spans.get(i)) {
        scroll = to_visible(scroll, area.width, *x, *w).min(max);
    }
    for ((line, warn), (x, _)) in chips.iter().zip(&spans) {
        let base = if *warn {
            theme.cc.warning
        } else {
            theme.foreground
        };
        let at = i32::from(area.x) + i32::from(*x) - i32::from(scroll);
        draw_at(buf, area, at, line, base);
    }
    scroll
}

/// Draw `line` starting at column `at` (maybe left of `area`), clipped.
fn draw_at(buf: &mut Buffer, area: Rect, at: i32, line: &Line, base: crate::empty_inbox::Rgb) {
    let skip = (i32::from(area.x) - at).max(0) as usize;
    let x = at.max(i32::from(area.x)) as u16;
    let mut cut = Vec::new();
    let mut seen = 0usize;
    for span in line {
        let mut text = String::new();
        for c in span.text.chars() {
            if seen >= skip {
                text.push(c);
            }
            seen += super::text::char_width(c);
        }
        cut.push(super::text::Span {
            text,
            look: span.look,
        });
    }
    canvas::line(buf, area, x, area.y, &cut, base);
}

/// The least sideways scroll that shows `x..x + w` (Textual's
/// `get_scroll_to_visible` on the x axis).
fn to_visible(scroll: u16, window: u16, x: u16, w: u16) -> u16 {
    let w = w.min(window);
    if x >= scroll && x + w <= scroll + window {
        return scroll;
    }
    let left = i32::from(x) - i32::from(scroll);
    let right = i32::from(x) - (i32::from(scroll) + i32::from(window) - i32::from(w));
    let delta = if right.abs() < left.abs() {
        right
    } else {
        left
    };
    (i32::from(scroll) + delta).max(0) as u16
}

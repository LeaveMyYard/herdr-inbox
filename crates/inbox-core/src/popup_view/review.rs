//! The open group's Submit tab (Python `ReviewView`): "Review your
//! answers", each open question with its draft (→ in success) or
//! "→ (not answered)", the warning when some are blank, then Submit
//! answers / Cancel.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::Ctx;
use super::fmt::{answer_line, first_line, tab_name};
use super::layout;
use super::marks::option;
use super::scroll::{self, Frame, Item};
use super::text::{Line, Ln, Look};
use crate::popup::{Popup, ReviewCursor, ReviewView};

const HINT: &str =
    "Enter to select · ↑/↓ to navigate · Tab/Arrow keys to switch questions · Esc to cancel";

/// Draw the Submit tab into the body `area`; `offset` is its scroll.
pub(crate) fn draw(
    buf: &mut Buffer,
    mut area: Rect,
    popup: &Popup,
    v: &ReviewView,
    ctx: &Ctx,
    offset: &mut u16,
) {
    let t = &ctx.theme;
    layout::hint(buf, &mut area, &Ln::new().plain(HINT).done(), t.muted);
    let frame = Frame {
        base: t.foreground,
        gutter: 0,
        theme: t,
    };
    scroll::draw(
        buf,
        area,
        &[Item::Text(body(popup, v, ctx))],
        &[],
        offset,
        &frame,
    );
}

fn body(popup: &Popup, v: &ReviewView, ctx: &Ctx) -> Vec<Line> {
    let t = &ctx.theme;
    let bold = Look::default().with_bold();
    let mut lines = vec![Ln::new().on(bold, "Review your answers").done(), Vec::new()];
    let data = popup.data();
    let items: Vec<_> = v.items.iter().filter_map(|&id| data.question(id)).collect();
    for q in &items {
        let mark = if v.cursor == Some(ReviewCursor::Item(q.id)) {
            Ln::new().on(Look::fg(t.cc.suggestion), "❯")
        } else {
            Ln::new().on(Look::fg(t.foreground), "●")
        };
        let text = format!(
            "{} · {} · {}",
            q.r#ref(),
            tab_name(Some(q)),
            first_line(&q.question, 80)
        );
        lines.push(mark.plain(" ").on(Look::fg(t.foreground), &text).done());
        lines.push(
            match &q.draft {
                Some(d) => Ln::new()
                    .plain("  ")
                    .on(Look::fg(t.cc.success), &format!("→ {}", answer_line(d))),
                None => Ln::new().plain("  ").on(Look::dimmed(), "→ (not answered)"),
            }
            .done(),
        );
    }
    if items.iter().filter(|q| q.draft.is_some()).count() < items.len() {
        lines.push(Vec::new());
        lines.push(
            Ln::new()
                .on(
                    Look::fg(t.cc.warning),
                    "⚠ You have not answered all questions",
                )
                .done(),
        );
    }
    lines.extend([
        Vec::new(),
        Ln::new().on(bold, "Ready to submit your answers?").done(),
        Vec::new(),
    ]);
    for (k, (target, label)) in [
        (ReviewCursor::Submit, "Submit answers"),
        (ReviewCursor::Cancel, "Cancel"),
    ]
    .iter()
    .enumerate()
    {
        lines.push(option(v.cursor == Some(*target), &format!("{}.", k + 1), label, t).done());
    }
    lines
}

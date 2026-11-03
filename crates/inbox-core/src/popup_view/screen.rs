//! The popup's screen (Python `InboxApp.compose` and its CSS): padding 1 2,
//! the group's title and tab row, the view in the rest, the radar and
//! deep-link hints under it, and the dismiss confirmation over it all.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::layout::{self, Stack};
use super::text::{Ln, Look};
use super::{
    Ctx, ViewState, alerts, canvas, chips, confirm, empty, fmt, list, question, review, small,
};
use crate::popup::{Entry, Mode, Popup, View};

/// Python `RADAR_HINT` and `HELD_HINT`.
const RADAR_HINT: &str =
    "herdr-radar is installed but ?N isn't in its sidebar — run the radar-sidebar action";
const HELD_HINT: &str =
    "Opened from another plugin — press Tab or → to start answering (Esc for the list)";

pub(crate) fn render(popup: &Popup, ctx: &Ctx, state: &mut ViewState, buf: &mut Buffer) {
    let t = &ctx.theme;
    let area = buf.area;
    canvas::clear(buf, area, t.background);
    let view_at = popup.timers().view_at;
    if popup.mode() == Mode::Empty {
        let since = ctx.now.saturating_sub(view_at.unwrap_or(ctx.now));
        let live = popup
            .entries()
            .iter()
            .filter(|e| !popup.closed().contains(e))
            .count();
        return empty::draw(buf, since as f64 / 1000.0, live);
    }
    let inner = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let mut stack = Stack::new(inner);
    if let Some(title) = chips::title(popup) {
        chips::draw_title(buf, &mut stack, &title, t);
    }
    if let Some(row) = chips::chips(popup, t) {
        let current = popup
            .current()
            .and_then(|c| popup.all_tabs().iter().position(|t| *t == c))
            .map(|i| i + 1);
        state.chips_x = chips::draw_chips(buf, &mut stack, &row, current, state.chips_x, t);
    }
    let mut below = Vec::new();
    if ctx.radar_missing {
        below.push((Ln::new().plain(RADAR_HINT).done(), Look::fg(t.muted)));
    }
    if popup.held() {
        below.push((
            Ln::new().plain(HELD_HINT).done(),
            Look::bold(t.cc.suggestion),
        ));
    }
    let below_rows: Vec<_> = below
        .iter()
        .map(|(l, look)| {
            let styled: Vec<_> = l
                .iter()
                .cloned()
                .map(|mut s| {
                    s.look.bold = look.bold;
                    s
                })
                .collect();
            (
                layout::rows(&[styled], inner.width),
                look.fg.unwrap_or(t.foreground),
            )
        })
        .collect();
    let under: u16 = below_rows.iter().map(|(r, _)| r.len() as u16).sum();
    let body = stack.rest((0, under));
    let mut y = body.bottom();
    for (rows, base) in &below_rows {
        let h = (rows.len() as u16).min(inner.bottom().saturating_sub(y));
        layout::draw(buf, Rect::new(inner.x, y, inner.width, h), rows, *base);
        y += h;
    }
    if state.body.0 != view_at {
        state.body = (view_at, 0);
    }
    let offset = &mut state.body.1;
    match popup.view() {
        View::List(v) => list::draw(buf, body, popup, v, ctx, offset),
        View::Question(v) => question::draw(buf, body, popup, v, ctx, offset),
        View::Review(v) => review::draw(buf, body, popup, v, ctx, offset),
        View::Gone(v) => small::gone(buf, body, v, ctx),
        View::Failed(v) => small::failed(buf, body, popup, v, ctx),
        View::Alert(v) => alerts::view(buf, body, popup, v, ctx),
        _ => {}
    }
    if let Some(c) = popup.confirm() {
        confirm::fade(buf, inner, t);
        confirm::draw(buf, inner, &c.what, &confirm_detail(popup), t);
    }
}

/// The confirmation's dim line: the first line of the question it names
/// (the open tab's, or the first open one of the list row's group).
fn confirm_detail(popup: &Popup) -> String {
    let data = popup.data();
    let q = match (popup.mode(), popup.view()) {
        (Mode::Group, _) => popup.current().and_then(|t| match t {
            crate::popup::Tab::Question(id) => data.question(id),
            crate::popup::Tab::Submit => None,
        }),
        (Mode::List, View::List(v)) => match &v.cursor {
            Some(Entry::Group(g)) => data.open_group(*g).first().copied(),
            _ => None,
        },
        _ => None,
    };
    q.map_or_else(String::new, |q| fmt::first_line(&q.question, 70))
}

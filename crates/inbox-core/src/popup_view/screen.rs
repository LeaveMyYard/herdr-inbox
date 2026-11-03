//! The popup's screen (Python `InboxApp.compose` and its CSS): padding 1 2,
//! the group's title and tab row, the view in the rest, the radar and
//! deep-link hints under it, and the dismiss confirmation over it all.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::layout::{self, Stack};
use super::text::{Line, Ln, Look};
use super::{
    Ctx, ViewState, alerts, canvas, chips, confirm, empty, fmt, list, question, review, small,
};
use crate::empty_inbox::Rgb;
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
    let inner = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let hints = hints(popup, ctx, inner.width);
    let under: u16 = hints.iter().map(|(r, _)| r.len() as u16).sum();
    if popup.mode() == Mode::Empty {
        let since = ctx.now.saturating_sub(view_at.unwrap_or(ctx.now));
        let live = popup
            .entries()
            .iter()
            .filter(|e| !popup.closed().contains(e))
            .count();
        empty::draw(buf, since as f64 / 1000.0, live);
        // the hints stay under the empty view, as Python's (over the scene's foot when short)
        return draw_hints(buf, inner, inner.bottom().saturating_sub(under), &hints);
    }
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
    let body = stack.rest((0, under));
    draw_hints(buf, inner, body.bottom(), &hints);
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
        View::None => {}
    }
    if let Some(c) = popup.confirm() {
        confirm::fade(buf, inner, t);
        confirm::draw(buf, inner, &c.what, &confirm_detail(popup), t);
    }
}

/// Python's `#radar-hint` and `#held-hint`, as rows `width` cells wide
/// with their widget colours.
fn hints(popup: &Popup, ctx: &Ctx, width: u16) -> Vec<(Vec<Line>, Rgb)> {
    let t = &ctx.theme;
    let hint = |line: Line| layout::rows(&[line], width);
    let mut out = Vec::new();
    if ctx.radar_missing {
        out.push((hint(Ln::new().plain(RADAR_HINT).done()), t.muted));
    }
    if popup.held() {
        let bold = Look::default().with_bold();
        out.push((hint(Ln::new().on(bold, HELD_HINT).done()), t.cc.suggestion));
    }
    out
}

/// Draw the hints from row `y` down, within `inner`.
fn draw_hints(buf: &mut Buffer, inner: Rect, mut y: u16, hints: &[(Vec<Line>, Rgb)]) {
    for (rows, base) in hints {
        let h = (rows.len() as u16).min(inner.bottom().saturating_sub(y));
        layout::draw(buf, Rect::new(inner.x, y, inner.width, h), rows, *base);
        y += h;
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

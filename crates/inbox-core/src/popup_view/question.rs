//! One question as Claude Code shows it (Python `QuestionView`): its header
//! chip (alone), who asked, the question and its context, then ❯ over the
//! numbered options, "Type something." and "Chat about this" (or Submit /
//! Next when several may be picked), notes under options, the open text
//! box, and a preview pane beside the options when they carry previews.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::Ctx;
use super::canvas::{self, Ink};
use super::fmt::meta;
use super::input::{self, Field};
use super::layout::{self, Stack};
use super::marks::{cursor, focused, number};
use super::scroll::{self, Frame, Item};
use super::text::{Line, Ln, Look, lines_of};
use crate::popup::{Editing, Popup, Question, QuestionView, Row};

/// Python `OTHER` and `CHAT`, and Claude Code's tick.
const OTHER: &str = "Type something.";
const CHAT: &str = "Chat about this";
const TICK: &str = "✔";

/// Draw the question view into the body `area`; `offset` is the options' scroll.
pub(crate) fn draw(
    buf: &mut Buffer,
    mut area: Rect,
    popup: &Popup,
    v: &QuestionView,
    ctx: &Ctx,
    offset: &mut u16,
) {
    let t = &ctx.theme;
    layout::hint(buf, &mut area, &Ln::new().plain(&hint(v)).done(), t.muted);
    let Some(q) = popup.view_question(v.qid) else {
        return;
    };
    let mut stack = Stack::new(area);
    if let Some(h) = q.header.as_deref().filter(|h| !h.is_empty() && !v.in_group) {
        let chip = Look {
            reverse: true,
            bold: true,
            ..Look::default()
        };
        layout::text(
            buf,
            &mut stack,
            &[Ln::new().on(chip, &format!(" {h} ")).done()],
            (0, 1),
            t.foreground,
        );
    }
    layout::text(
        buf,
        &mut stack,
        &lines_of(&meta(q), Look::default()),
        (0, 0),
        t.muted,
    );
    layout::text(
        buf,
        &mut stack,
        &lines_of(&q.question, Look::default().with_bold()),
        (1, 1),
        t.foreground,
    );
    if let Some(c) = q.context.as_deref().filter(|c| !c.is_empty()) {
        layout::text(
            buf,
            &mut stack,
            &lines_of(c, Look::default()),
            (0, 1),
            t.muted,
        );
    }
    let (items, visible) = items(q, v, ctx);
    let frame = Frame {
        base: t.foreground,
        gutter: 0,
        theme: t,
    };
    if !q.has_preview() {
        return scroll::draw(buf, stack.rest((0, 0)), &items, &visible, offset, &frame);
    }
    let split = stack.rest((0, 1));
    let left = split.width / 2;
    scroll::draw(
        buf,
        Rect {
            width: left,
            ..split
        },
        &items,
        &visible,
        offset,
        &frame,
    );
    let pane = Rect {
        x: split.x + left,
        width: split.width - left,
        ..split
    };
    preview(buf, pane, q, v, ctx);
}

/// The preview pane: a rule in suggestion, the focused option's preview.
fn preview(buf: &mut Buffer, pane: Rect, q: &Question, v: &QuestionView, ctx: &Ctx) {
    let t = &ctx.theme;
    let rule = Ink {
        fg: Some(t.cc.suggestion),
        bg: Some(t.background),
        bold: false,
        italic: false,
    };
    for y in pane.top()..pane.bottom() {
        canvas::put(buf, pane, pane.x, y, "│", rule);
    }
    let text = match v.rows.get(v.cursor) {
        Some(Row::Option(i)) => q.options[*i].preview.as_deref().filter(|p| !p.is_empty()),
        _ => None,
    };
    let (text, base) = match text {
        Some(p) => (p, t.foreground),
        None => ("No preview available", t.muted),
    };
    let inner = Rect {
        x: pane.x + 2,
        width: pane.width.saturating_sub(2),
        ..pane
    };
    let frame = Frame {
        base,
        gutter: 1,
        theme: t,
    };
    scroll::draw(
        buf,
        inner,
        &[Item::Text(lines_of(text, Look::default()))],
        &[],
        &mut 0,
        &frame,
    );
}

/// Python `hint`.
fn hint(v: &QuestionView) -> String {
    if v.editing.is_some() {
        return "Enter to save · Esc to cancel".to_string();
    }
    let nav = if v.in_group {
        "Tab/Arrow keys to navigate"
    } else {
        "↑/↓ to navigate"
    };
    let pick = if v.multi_select {
        "Enter to toggle"
    } else {
        "Enter to select"
    };
    format!("{pick} · {nav} · press n to add notes · g go to agent · d dismiss · Esc to cancel")
}

/// The rows, with the open text box after its row; the items to keep in view.
fn items(q: &Question, v: &QuestionView, ctx: &Ctx) -> (Vec<Item>, Vec<usize>) {
    let mut items = Vec::new();
    let mut visible = Vec::new();
    for (i, row) in v.rows.iter().enumerate() {
        if i == v.cursor {
            visible.push(items.len());
        }
        items.push(Item::Text(row_text(q, v, i, ctx)));
        let boxed = match (&v.editing, row) {
            (Some(Editing::Note(k, b)), Row::Option(j)) if k == j => Some((b, "Notes")),
            (Some(Editing::Text(b)), Row::Other) => Some((b, OTHER)),
            _ => None,
        };
        if let Some((b, placeholder)) = boxed {
            let (value, at, selected, theme) = (b.value(), b.cursor(), b.selected(), ctx.theme);
            let focused = v.box_focused;
            let rows = move |w: u16| {
                let field = Field {
                    value: &value,
                    cursor: at,
                    placeholder,
                    selected,
                    focused,
                };
                input::rows(&field, w, theme.background, &theme)
            };
            visible.push(items.len());
            items.push(Item::Fixed {
                indent: 4,
                rows: Box::new(rows),
                height: 3,
            });
        }
    }
    (items, visible)
}

/// Python `row_text`: one row's lines.
fn row_text(q: &Question, v: &QuestionView, i: usize, ctx: &Ctx) -> Vec<Line> {
    let t = &ctx.theme;
    let here = i == v.cursor;
    let head = cursor(here, t).plain(" ");
    let num = number(&format!("{}.", i + 1), t);
    let success = Look::fg(t.cc.success);
    let tick = Ln::new().plain("\u{a0}").on(success, TICK);
    let boxed = |on: bool| match (v.multi_select, on) {
        (false, _) => Ln::new(),
        (true, true) => Ln::new().on(success, &format!("[{TICK}]")).plain("\u{a0}"),
        (true, false) => Ln::new().plain("[ ]\u{a0}"),
    };
    match v.rows[i] {
        Row::Submit => {
            let label = if v.last { "Submit" } else { "Next" };
            vec![
                head.push(number("  ", t))
                    .plain(" ")
                    .push(focused(here, label, t))
                    .done(),
            ]
        }
        Row::Chat => vec![
            head.push(num)
                .plain(" ")
                .push(focused(here, CHAT, t))
                .done(),
        ],
        Row::Other => {
            let text = v.text.as_deref().filter(|s| !s.is_empty());
            let label = match (here, text) {
                (true, _) => Ln::new().on(Look::bold(t.cc.suggestion), text.unwrap_or(OTHER)),
                (false, Some(s)) => Ln::new().on(Look::italic(), s),
                (false, None) => Ln::new().on(Look::dimmed(), OTHER),
            };
            let mut line = head
                .push(num)
                .plain(" ")
                .push(boxed(text.is_some()))
                .push(label);
            if text.is_some() && !v.multi_select {
                line = line.push(tick);
            }
            vec![line.done()]
        }
        Row::Option(k) => {
            let opt = &q.options[k];
            let picked = v.picked.contains(&k);
            let mut line = head
                .push(num)
                .plain(" ")
                .push(boxed(picked))
                .push(focused(here, &opt.label, t));
            if picked && !v.multi_select {
                line = line.push(tick);
            }
            let mut lines = vec![line.done()];
            if let Some(d) = opt.description.as_deref().filter(|d| !d.is_empty()) {
                lines.push(Ln::new().plain("      ").on(Look::dimmed(), d).done());
            }
            if let Some(n) = v.notes.get(&k).filter(|n| !n.is_empty()) {
                let notes = Ln::new()
                    .plain("      ")
                    .on(Look::fg(t.cc.suggestion), "Notes:")
                    .plain(" ");
                lines.push(notes.on(Look::italic(), n).done());
            }
            lines
        }
    }
}

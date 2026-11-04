//! Alerts: an agent's "please check X" (⚑) or an agent waiting at a prompt
//! (⏸), as a list row (Python `_alert_row`) and as their own view
//! (`AlertView`): no numbers and no Enter, `d` acknowledges, `g` goes there.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::Ctx;
use super::fmt::age;
use super::layout::{self, Stack};
use super::text::{Line, Ln, Look, lines_of};
use super::theme::Theme;
use crate::empty_inbox::Rgb;
use crate::popup::{Alert, AlertView, Popup};

/// Python `SEVERITY_STYLE`: `info` in suggestion, the rest in warning.
fn severity(a: &Alert, t: &Theme) -> Rgb {
    if a.severity == "info" {
        t.cc.suggestion
    } else {
        t.cc.warning
    }
}

fn mark(a: &Alert) -> &'static str {
    if a.kind == "blocked" { "⏸" } else { "⚑" }
}

fn place(a: &Alert) -> Option<String> {
    a.tab_label
        .clone()
        .filter(|s| !s.is_empty())
        .or(a.pane_id.clone().filter(|s| !s.is_empty()))
}

/// Who and when: tab (or pane), asker, age.
fn meta(a: &Alert, ctx: &Ctx) -> String {
    let parts = [
        place(a),
        a.asked_by.clone(),
        Some(age(&a.created_at, ctx.wall)),
    ];
    let parts: Vec<String> = parts
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    parts.join(" · ")
}

/// An alert's list row, after its `head` (cursor and the blank number).
pub(crate) fn row(
    popup: &Popup,
    id: i64,
    here: bool,
    head: Ln,
    closed: Option<&String>,
    ctx: &Ctx,
) -> Vec<Line> {
    let t = &ctx.theme;
    let a = popup.data().alert(id);
    let open = a.filter(|a| a.status == "open");
    let (Some(a), None) = (open, closed) else {
        let why = closed
            .cloned()
            .unwrap_or_else(|| a.map_or("gone".to_string(), |a| a.status.clone()));
        return vec![head.on(Look::dimmed(), &format!("✓ A{id} · {why}")).done()];
    };
    let style = severity(a, t);
    let text = format!("{} A{id} · {} · {}", mark(a), a.severity, a.title);
    let line1 = head.on(
        if here {
            Look::bold(style)
        } else {
            Look::fg(style)
        },
        &text,
    );
    let tail = if a.kind == "blocked" {
        "g goes there · d dismisses"
    } else {
        "d acknowledges · g goes there"
    };
    let detail = Ln::new()
        .plain("      ")
        .on(Look::dimmed(), &format!("{} · {tail}", meta(a, ctx)));
    vec![line1.done(), detail.done()]
}

/// The alert's own view, in the body `area`.
pub(crate) fn view(buf: &mut Buffer, mut area: Rect, popup: &Popup, view: &AlertView, ctx: &Ctx) {
    let t = &ctx.theme;
    let Some(a) = popup.data().alert(view.alert) else {
        return;
    };
    let (what, keys) = if a.kind == "blocked" {
        (
            "the agent is waiting at a prompt in its pane; answer it there",
            "g go to the agent · d dismiss (it shows again at its next prompt) · Esc back",
        )
    } else {
        (
            "no answer expected; the agent sees your acknowledgement in check_answers",
            "d acknowledge · g go to the agent · Esc back",
        )
    };
    layout::hint(buf, &mut area, &Ln::new().plain(keys).done(), t.muted);
    let mut stack = Stack::new(area);
    let head = Ln::new()
        .on(
            Look::bold(severity(a, t)),
            &format!("{} A{} · {}", mark(a), a.id, a.severity),
        )
        .plain(": ")
        .on(Look::default().with_bold(), &a.title);
    layout::text(buf, &mut stack, &[head.done()], (1, 1), t.foreground);
    layout::text(
        buf,
        &mut stack,
        &lines_of(&meta(a, ctx), Look::default()),
        (0, 0),
        t.muted,
    );
    if let Some(m) = a.message.as_deref().filter(|m| !m.is_empty()) {
        layout::text(
            buf,
            &mut stack,
            &lines_of(m, Look::default()),
            (1, 1),
            t.foreground,
        );
    }
    layout::text(
        buf,
        &mut stack,
        &lines_of(what, Look::dimmed()),
        (0, 0),
        t.muted,
    );
}

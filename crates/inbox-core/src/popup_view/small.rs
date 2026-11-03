//! The two small views: an inert placeholder for what closed under the
//! operator (Python `GoneView`) and an undelivered answer's Retry / Stop
//! showing it (`FailedView`).

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::Ctx;
use super::fmt::{pending_reason, pending_text, tab_name};
use super::layout::{self, Stack};
use super::marks::option;
use super::text::{Line, Ln, Look, lines_of};
use crate::popup::{AGENT_GONE, FailedView, GoneView, Popup, Question};

/// "Q3 was closed (withdrawn by the agent)", in the body `area`.
pub(crate) fn gone(buf: &mut Buffer, mut area: Rect, view: &GoneView, ctx: &Ctx) {
    let t = &ctx.theme;
    let hint = "Tab/Arrow keys to move to another question · Esc to cancel";
    layout::hint(buf, &mut area, &Ln::new().plain(hint).done(), t.muted);
    let mut stack = Stack::new(area);
    let head = Ln::new()
        .on(
            Look::bold(t.cc.warning),
            &format!("{} was closed", view.r#ref),
        )
        .plain(" ")
        .on(Look::dimmed(), &format!("({})", view.reason));
    layout::text(buf, &mut stack, &[head.done()], (1, 1), t.foreground);
    let meta = Ln::new().plain("Nothing to answer here.").done();
    layout::text(buf, &mut stack, &[meta], (0, 0), t.muted);
}

/// `Q7 · tab · asker`.
pub(crate) fn meta(q: &Question) -> String {
    let mut meta = format!("{} · {}", q.r#ref(), tab_name(Some(q)));
    if let Some(by) = q.asked_by.as_deref().filter(|b| !b.is_empty()) {
        meta.push_str(&format!(" · {by}"));
    }
    meta
}

/// Why the answer is here, as the view's first line.
fn state(q: &Question, ctx: &Ctx) -> Line {
    let t = &ctx.theme;
    let err = q.delivery_error.as_deref().unwrap_or("");
    let (head, look, why) = if q.agent_gone() {
        let why = err.strip_prefix(AGENT_GONE).unwrap_or(err).to_string();
        let head = format!(
            "⚠ {} agent closed — answer kept for check_answers",
            q.r#ref()
        );
        (head, Look::fg(t.cc.warning), why)
    } else if let Some(reason) = pending_reason(q) {
        let why = pending_text(reason, ctx.busy_for.get(&q.id).copied().unwrap_or(0.0));
        let head = format!(
            "⏳ {} is already answered — waiting to be delivered",
            q.r#ref()
        );
        (head, Look::default().with_bold(), why)
    } else {
        (
            format!("⚠ {} not delivered", q.r#ref()),
            Look::fg(t.cc.warning),
            err.to_string(),
        )
    };
    Ln::new()
        .on(look, &head)
        .plain(": ")
        .on(Look::dimmed(), &why)
        .done()
}

/// An undelivered answer's view, in the body `area`.
pub(crate) fn failed(
    buf: &mut Buffer,
    mut area: Rect,
    popup: &Popup,
    view: &FailedView,
    ctx: &Ctx,
) {
    let t = &ctx.theme;
    let hint = "Enter to select · Tab/Arrow keys to navigate · g go to agent · Esc to cancel";
    layout::hint(buf, &mut area, &Ln::new().plain(hint).done(), t.muted);
    let Some(q) = popup.data().question(view.qid) else {
        return;
    };
    let mut stack = Stack::new(area);
    layout::text(buf, &mut stack, &[state(q, ctx)], (1, 1), t.foreground);
    layout::text(
        buf,
        &mut stack,
        &lines_of(&meta(q), Look::default()),
        (0, 0),
        t.muted,
    );
    let mut body = lines_of(&q.question, Look::default().with_bold());
    let answer = format!("→ {}", q.answer.as_deref().unwrap_or(""));
    body.extend(
        lines_of(&answer, Look::fg(t.cc.success))
            .into_iter()
            .enumerate()
            .map(|(i, mut l)| {
                if i == 0 {
                    l.insert(0, Ln::new().plain("  ").done().remove(0));
                }
                l
            }),
    );
    layout::text(buf, &mut stack, &body, (1, 1), t.foreground);
    let busy = pending_reason(q).is_some_and(|r| r.starts_with("busy:"));
    let retry = if busy {
        "Send now (the agent is busy; the transcript check confirms it landed)"
    } else {
        "Retry delivery"
    };
    let stop = "Stop showing it here (the agent can still fetch it with check_answers)";
    let rows: Vec<Line> = [retry, stop]
        .iter()
        .enumerate()
        .map(|(k, label)| option(k == view.cursor, &format!("{}.", k + 1), label, t).done())
        .collect();
    layout::text(buf, &mut stack, &rows, (0, 0), t.foreground);
}

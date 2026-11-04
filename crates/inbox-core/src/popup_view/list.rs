//! The group list (Python `GroupListView`): "Open questions · …", one
//! two-line row per group, ⚠ answer, gone-agents summary or alert (alerts
//! unnumbered), placeholders for rows that ended, the window on the cursor.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fmt::{age, first_line, group_name, pending_reason, pending_text, tab_name};
use super::layout::{self, Stack};
use super::marks::{cursor, focused, number};
use super::scroll::{self, Frame, Item};
use super::text::{Line, Ln, Look};
use super::{Ctx, alerts};
use crate::popup::{Entry, ListView, Popup};

const HINT: &str = "Enter to open · ↑/↓ to navigate · g go to agent · d dismiss · Esc to cancel";

/// Draw the list into the body `area`; `offset` is its scroll.
pub(crate) fn draw(
    buf: &mut Buffer,
    mut area: Rect,
    popup: &Popup,
    view: &ListView,
    ctx: &Ctx,
    offset: &mut u16,
) {
    let t = &ctx.theme;
    layout::hint(buf, &mut area, &Ln::new().plain(HINT).done(), t.muted);
    let mut stack = Stack::new(area);
    layout::text(buf, &mut stack, &[title(popup)], (1, 1), t.foreground);
    let rows = stack.rest((0, 0));
    let frame = Frame {
        base: t.foreground,
        gutter: 0,
        theme: t,
    };
    let entries = popup.entries();
    if entries.is_empty() {
        let empty = Ln::new().plain("No open questions").done();
        return scroll::draw(buf, rows, &[Item::Text(vec![empty])], &[], offset, &frame);
    }
    let mut n = 0;
    let items: Vec<Item> = entries
        .iter()
        .map(|e| {
            let numbered = !e.is_alert();
            n += usize::from(numbered);
            let num = numbered.then(|| format!("{n}."));
            Item::Text(row(
                popup,
                e,
                view.cursor.as_ref() == Some(e),
                num.as_deref(),
                ctx,
            ))
        })
        .collect();
    let visible: Vec<usize> = entries
        .iter()
        .position(|e| view.cursor.as_ref() == Some(e))
        .into_iter()
        .collect();
    scroll::draw(buf, rows, &items, &visible, offset, &frame);
}

/// "Open questions · 3 groups · 1 pending delivery · 1 not delivered".
fn title(popup: &Popup) -> Line {
    let live: Vec<&Entry> = popup
        .entries()
        .iter()
        .filter(|e| !popup.closed().contains(e))
        .collect();
    let groups = live.iter().filter(|e| matches!(e, Entry::Group(_))).count();
    let failed: Vec<Option<&crate::popup::Question>> = live
        .iter()
        .filter_map(|e| match e {
            Entry::Failed(id) => Some(popup.data().question(*id)),
            _ => None,
        })
        .collect();
    let pending = failed
        .iter()
        .filter(|q| q.is_some_and(|q| pending_reason(q).is_some()))
        .count();
    let gone = if live.contains(&&Entry::Gone) {
        popup.gone_live().len()
    } else {
        0
    };
    let not_delivered = failed.len() - pending + gone;
    let mut parts = vec![plural(groups, "group")];
    if pending > 0 {
        parts.push(format!("{pending} pending delivery"));
    }
    if not_delivered > 0 {
        parts.push(format!("{not_delivered} not delivered"));
    }
    Ln::new()
        .on(Look::default().with_bold(), "Open questions")
        .plain(" ")
        .on(Look::dimmed(), &format!("· {}", parts.join(" · ")))
        .done()
}

pub(crate) fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// One entry's lines (Python `row_text`).
fn row(popup: &Popup, entry: &Entry, here: bool, num: Option<&str>, ctx: &Ctx) -> Vec<Line> {
    let t = &ctx.theme;
    let num = num.map_or_else(|| Ln::new().plain("  "), |n| number(n, t));
    let head = cursor(here, t).plain(" ").push(num).plain(" ");
    let closed = popup.closed().get(entry);
    let warn = |text: &str| {
        let look = if here {
            Look::bold(t.cc.warning)
        } else {
            Look::fg(t.cc.warning)
        };
        Ln::new().on(look, text)
    };
    let detail = |text: &str| Ln::new().plain("      ").on(Look::dimmed(), text).done();
    let data = popup.data();
    match entry {
        Entry::Alert(id) => alerts::row(popup, *id, here, head, closed, ctx),
        Entry::Gone => {
            if let Some(why) = closed {
                return vec![
                    head.on(
                        Look::fg(t.cc.warning),
                        &format!("✕ answers for gone agents · {why}"),
                    )
                    .done(),
                ];
            }
            let ids = popup.gone_live();
            let s = if ids.len() == 1 { "" } else { "s" };
            let line = format!(
                "⚠ {} answer{s} for agents that are gone · d hides them all",
                ids.len()
            );
            let refs: Vec<String> = ids.iter().take(8).map(|id| format!("Q{id}")).collect();
            let more = if ids.len() > 8 { " …" } else { "" };
            let refs = format!("{}{more}", refs.join(", "));
            vec![
                head.push(warn(&line)).done(),
                detail(&format!("{refs} · still readable with check_answers")),
            ]
        }
        Entry::Failed(id) => {
            let q = data.question(*id);
            let where_ = tab_name(q);
            if let Some(why) = closed {
                return vec![
                    head.on(
                        Look::fg(t.cc.warning),
                        &format!("✕ Q{id} · {where_} · {why}"),
                    )
                    .done(),
                ];
            }
            let Some(q) = q else { return vec![head.done()] };
            let reason = pending_reason(q);
            let text =
                reason.map(|r| pending_text(r, ctx.busy_for.get(id).copied().unwrap_or(0.0)));
            let line1 = if q.agent_gone() {
                warn(&format!(
                    "⚠ Q{id} agent closed — answer kept for check_answers · {where_}"
                ))
            } else if let Some(rest) = text.as_deref().and_then(|t| t.strip_prefix("⚠ ")) {
                warn(&format!("⚠ Q{id} answered · {rest} · {where_}"))
            } else if let Some(text) = text.as_deref() {
                focused(
                    here,
                    &format!("⏳ Q{id} answered · waiting to deliver: {text} · {where_}"),
                    t,
                )
            } else {
                let err = q
                    .delivery_error
                    .as_deref()
                    .filter(|e| !e.is_empty())
                    .unwrap_or("no attempt recorded");
                warn(&format!("⚠ Q{id} not delivered · {where_} · {err}"))
            };
            let answer = first_line(q.answer.as_deref().unwrap_or(""), 40);
            vec![
                head.push(line1).done(),
                detail(&format!("{} → {answer}", first_line(&q.question, 60))),
            ]
        }
        Entry::Group(g) => {
            let qs = data.group(*g);
            let first = qs.first().copied();
            let where_ = tab_name(first);
            let live: Vec<_> = qs.iter().copied().filter(|q| q.status == "open").collect();
            let name = match first {
                Some(_) => group_name(if live.is_empty() { &qs } else { &live }),
                None => where_.clone(),
            };
            let (Some(first), None) = (first, closed) else {
                let why = closed.map_or("closed", String::as_str);
                return vec![
                    head.on(
                        Look::fg(t.cc.warning),
                        &format!("✕ {name} · {where_} · {why}"),
                    )
                    .done(),
                ];
            };
            let drafted = live.iter().filter(|q| q.draft.is_some()).count();
            let mut line1 = head.push(focused(
                here,
                &format!("{name} · {}", plural(live.len(), "question")),
                t,
            ));
            line1 = if drafted > 0 {
                line1
                    .plain(" ")
                    .on(Look::fg(t.cc.success), &format!("· {drafted} drafted "))
            } else {
                line1.plain(" ")
            };
            line1 = line1.on(
                Look::dimmed(),
                &format!("· {}", age(&first.created_at, ctx.wall)),
            );
            let chips: Vec<String> = live
                .iter()
                .map(|q| {
                    q.header
                        .clone()
                        .filter(|h| !h.is_empty())
                        .unwrap_or_else(|| first_line(&q.question, 30))
                })
                .collect();
            let mut chips = chips.join(" · ");
            if live.len() == 1 && chips == name {
                chips.clear();
            }
            let mut parts = vec![where_];
            parts.extend(first.asked_by.clone().filter(|a| !a.is_empty()));
            if !chips.is_empty() {
                parts.push(chips);
            }
            vec![line1.done(), detail(&parts.join(" · "))]
        }
    }
}

//! A remote machine's list rows (Python `_remote_row`, `_outbox_row`,
//! `_problem_row`) and the open remote question's title line: what the
//! local mirror says, the machine's name in front of every ref.

use super::Ctx;
use super::fmt::{age, first_line};
use super::list::plural;
use super::marks::focused;
use super::text::{Line, Ln, Look};
use crate::popup::{Entry, Popup};

/// The rows of a remote entry, after its `head` (cursor and number).
pub(crate) fn row(
    popup: &Popup,
    entry: &Entry,
    here: bool,
    head: Ln,
    closed: Option<&String>,
    ctx: &Ctx,
) -> Vec<Line> {
    let t = &ctx.theme;
    let r = &popup.data().remote;
    let done = |head: Ln, what: &str| vec![head.on(Look::dimmed(), &format!("✓ {what}")).done()];
    let warn = |text: &str| {
        let look = if here {
            Look::bold(t.cc.warning)
        } else {
            Look::fg(t.cc.warning)
        };
        Ln::new().on(look, text)
    };
    match entry {
        Entry::Remote(key) => {
            let Some(g) = r.group(key).filter(|_| closed.is_none()) else {
                let mid = key.split('|').next().unwrap_or_default();
                let why = closed.map_or("closed", String::as_str);
                return done(head, &format!("{} · {why}", r.name(mid)));
            };
            let first = &g.questions[0].question;
            let label = r.name(&g.machine_id);
            let name = [&first.group_title, &first.header]
                .into_iter()
                .flatten()
                .find(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(|| first_line(&first.question, 80));
            let text = format!(
                "{label}:Q{} {name} · {}",
                first.id,
                plural(g.questions.len(), "question")
            );
            let mut line1 = head.push(focused(here, &text, t));
            if !first.created_at.is_empty() {
                line1 = line1.plain(" ").on(
                    Look::dimmed(),
                    &format!("· {}", age(&first.created_at, ctx.wall)),
                );
            }
            let mut parts = vec![label];
            parts.extend(
                [&first.tab_label, &first.asked_by]
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.is_empty())
                    .cloned(),
            );
            let mut detail = Ln::new()
                .plain("      ")
                .on(Look::dimmed(), &parts.join(" · "));
            if let Some(note) = r.machine(&g.machine_id).and_then(|m| m.offline.clone()) {
                let look = Look {
                    dim: true,
                    ..Look::fg(t.cc.warning)
                };
                detail = detail.on(Look::dimmed(), " · ").on(look, &note);
            }
            vec![line1.done(), detail.done()]
        }
        Entry::Outbox(id) => {
            let Some(o) = r.outbox_row(id).filter(|_| closed.is_none()) else {
                return done(head, closed.map_or("sent", String::as_str));
            };
            let label = r.name(&o.machine_id);
            let kind = if o.action == "ack_alert" { 'A' } else { 'Q' };
            let r#ref = format!("{label}:{kind}{}", o.remote_id);
            let action = &o.action;
            let line = match o.state.as_str() {
                "queued" if o.sending => {
                    let text = format!("⏳ {ref} {action} · sending to {label} now");
                    focused(here, &text, t)
                }
                "queued" if o.expired => warn(&format!(
                    "⚠ {ref} {action} expired · not sent to {label} · d clears"
                )),
                "queued" => {
                    let until = &o.expires;
                    let text =
                        format!("⏳ {ref} {action} queued · sending to {label} · expires {until}");
                    focused(here, &text, t)
                }
                state => {
                    let why = o.result.as_deref().unwrap_or("");
                    warn(&format!("⚠ {ref} {action} {state}: {why} · d clears"))
                }
            };
            vec![head.push(line).done()]
        }
        Entry::Problem(key) => {
            let Some(p) = r.problem(key).filter(|_| closed.is_none()) else {
                return done(head, closed.map_or("cleared", String::as_str));
            };
            let line1 = warn(&format!(
                "⚠ {}:Q{} answered · {}",
                p.label, p.remote_id, p.why
            ));
            let answer = first_line(p.answer.as_deref().unwrap_or(""), 40);
            let text = format!("{} → {answer} · d clears", first_line(&p.question, 60));
            let detail = Ln::new().plain("      ").on(Look::dimmed(), &text);
            vec![head.push(line1).done(), detail.done()]
        }
        _ => vec![head.done()],
    }
}

/// The open remote question's title row: `project1:Q3 · its title`.
pub(crate) fn title(popup: &Popup) -> Option<Line> {
    let (q, label) = popup.remote_question()?;
    let q = &q.question;
    let name = q
        .group_title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| first_line(&q.question, 80));
    let text = format!("{label}:Q{} · {name}", q.id);
    Some(Ln::new().plain(&text).done())
}

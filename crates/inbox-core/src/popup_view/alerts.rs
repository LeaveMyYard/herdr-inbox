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
use crate::popup::{Alert, AlertView, Entry, Popup};

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

/// Who and when: the machine (a remote alert's), tab (or pane), asker, age.
fn meta(a: &Alert, label: Option<&str>, ctx: &Ctx) -> String {
    let parts = [
        label.map(str::to_string),
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

/// An alert's list row, after its `head` (cursor and the blank number): a
/// local one (`entry` [`Entry::Alert`]) or a remote machine's
/// ([`Entry::RemoteAlert`], its ref `<label>:A<id>`).
pub(crate) fn row(
    popup: &Popup,
    entry: &Entry,
    here: bool,
    head: Ln,
    closed: Option<&String>,
    ctx: &Ctx,
) -> Vec<Line> {
    let t = &ctx.theme;
    let data = popup.data();
    let (a, r#ref, remote) = match entry {
        Entry::Alert(id) => (data.alert(*id), format!("A{id}"), false),
        Entry::RemoteAlert(key) => {
            // Python `_alert`: the open ones only; none found, no label ("None")
            let found = data.remote.alert(key);
            let label = found.map_or("None".to_string(), |a| data.remote.name(&a.machine_id));
            let aid = key.rsplit('|').next().unwrap_or_default();
            (found.map(|a| &a.alert), format!("{label}:A{aid}"), true)
        }
        _ => return vec![head.done()],
    };
    let open = a.filter(|a| a.status == "open");
    let (Some(a), None) = (open, closed) else {
        let why = closed
            .cloned()
            .unwrap_or_else(|| a.map_or("gone".to_string(), |a| a.status.clone()));
        return vec![head.on(Look::dimmed(), &format!("✓ {ref} · {why}")).done()];
    };
    let style = severity(a, t);
    let text = format!("{} {ref} · {} · {}", mark(a), a.severity, a.title);
    let line1 = head.on(
        if here {
            Look::bold(style)
        } else {
            Look::fg(style)
        },
        &text,
    );
    let tail = match (remote, a.kind == "blocked") {
        (true, true) => "open it from herdr's machine view",
        (true, false) => "d acknowledges",
        (false, true) => "g goes there · d dismisses",
        (false, false) => "d acknowledges · g goes there",
    };
    let detail = Ln::new()
        .plain("      ")
        .on(Look::dimmed(), &format!("{} · {tail}", meta(a, None, ctx)));
    vec![line1.done(), detail.done()]
}

/// The alert's own view, in the body `area`.
pub(crate) fn view(buf: &mut Buffer, mut area: Rect, popup: &Popup, view: &AlertView, ctx: &Ctx) {
    let t = &ctx.theme;
    let label = view.label.as_deref();
    let a = match label {
        Some(_) => popup.remote_alert().map(|r| &r.alert),
        None => popup.data().alert(view.alert),
    };
    let Some(a) = a else {
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
    let keys = if label.is_some() {
        // a remote machine's agent: never gone to from here
        keys.replace("g go to the agent · ", "")
            .replace(" · g go to the agent", "")
    } else {
        keys.to_string()
    };
    layout::hint(buf, &mut area, &Ln::new().plain(&keys).done(), t.muted);
    let mut stack = Stack::new(area);
    let head = Ln::new()
        .on(
            Look::bold(severity(a, t)),
            &format!(
                "{} {}A{} · {}",
                mark(a),
                label.map_or(String::new(), |l| format!("{l}:")),
                a.id,
                a.severity
            ),
        )
        .plain(": ")
        .on(Look::default().with_bold(), &a.title);
    layout::text(buf, &mut stack, &[head.done()], (1, 1), t.foreground);
    layout::text(
        buf,
        &mut stack,
        &lines_of(&meta(a, label, ctx), Look::default()),
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

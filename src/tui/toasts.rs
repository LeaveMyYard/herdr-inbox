//! The popup's toasts (Textual's `App.notify`): the model's
//! [`Effect::Notify`](inbox_core::popup::Effect::Notify)s and the IO layer's
//! own notices, each shown [`TIMEOUT`] and drawn over the popup as Textual's
//! toast rack draws them: stacked at the bottom right, newest last, each
//! `min(60, 50%)` cells wide with a coloured left edge, a cell of padding
//! and a blank line between toasts. The renderer draws no toasts; this
//! module owns their lifetime and their cells.

use inbox_core::popup::{Millis, Severity};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;

/// How long a toast shows (Textual `App.NOTIFICATION_TIMEOUT`, 5 s).
pub const TIMEOUT: Millis = 5000;

// Textual's `textual-dark` design: `$panel-lighten-1`, `$success`, `$warning`,
// `$foreground`.
const FILL: Color = Color::Rgb(0x34, 0x3f, 0x49);
const SUCCESS: Color = Color::Rgb(0x4e, 0xbf, 0x71);
const WARNING: Color = Color::Rgb(0xfe, 0xa6, 0x2b);
const TEXT: Color = Color::Rgb(0xe0, 0xe0, 0xe0);

/// One notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// Its text.
    pub text: String,
    /// Its severity (the edge's colour).
    pub severity: Severity,
    /// A bold first line, if any (Textual's `title`).
    pub title: Option<String>,
}

impl Toast {
    /// A notice without a title.
    pub fn new(text: impl Into<String>, severity: Severity) -> Toast {
        Toast {
            text: text.into(),
            severity,
            title: None,
        }
    }
}

/// The toasts on screen, each with the time it goes.
#[derive(Clone, Debug, Default)]
pub struct Toasts(Vec<(Toast, Millis)>);

impl Toasts {
    /// Shows `toast` from `now`.
    pub fn push(&mut self, toast: Toast, now: Millis) {
        self.0.push((toast, now + TIMEOUT));
    }

    /// Drops the toasts whose time is up.
    pub fn expire(&mut self, now: Millis) {
        self.0.retain(|(_, until)| *until > now);
    }

    /// The toasts on screen, oldest first.
    pub fn shown(&self) -> impl Iterator<Item = &Toast> {
        self.0.iter().map(|(t, _)| t)
    }

    /// When the next toast goes, if any.
    pub fn next_expiry(&self) -> Option<Millis> {
        self.0.iter().map(|(_, until)| *until).min()
    }

    /// Draws the toasts over `buf`, newest at the bottom; what does not fit
    /// above the oldest shown is left out.
    pub fn draw(&self, buf: &mut Buffer) {
        let area = buf.area;
        let width = (area.width / 2).min(60);
        if width < 4 || area.height < 3 {
            return;
        }
        let x = area.x + area.width - width;
        let mut bottom = area.y + area.height - 1; // the rack's bottom margin
        for (toast, _) in self.0.iter().rev() {
            let lines = lines_of(toast, usize::from(width - 3));
            let height = u16::try_from(lines.len())
                .unwrap_or(u16::MAX)
                .saturating_add(2);
            let Some(top) = bottom.checked_sub(height).filter(|t| *t >= area.y) else {
                break;
            };
            paint(
                buf,
                Rect::new(x, top, width, height),
                toast.severity,
                &lines,
            );
            match top.checked_sub(1) {
                Some(b) if b > area.y => bottom = b, // one line between toasts
                _ => break,
            }
        }
    }
}

/// A toast's lines: its title (bold), then its text folded to `width`.
fn lines_of(toast: &Toast, width: usize) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    if let Some(t) = &toast.title {
        out.extend(fold(t, width).into_iter().map(|l| (l, true)));
    }
    out.extend(fold(&toast.text, width).into_iter().map(|l| (l, false)));
    out
}

/// `text` folded at spaces to `width` cells (a longer word is cut).
fn fold(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let (mut line, mut used) = (String::new(), 0usize);
        for word in para.split(' ') {
            let w: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();
            let gap = usize::from(!line.is_empty());
            if used + gap + w > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            if !line.is_empty() {
                line.push(' ');
                used += 1;
            }
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += cw;
            }
        }
        lines.push(line);
    }
    lines
}

fn paint(buf: &mut Buffer, r: Rect, severity: Severity, lines: &[(String, bool)]) {
    let edge = match severity {
        Severity::Information => SUCCESS,
        Severity::Warning => WARNING,
    };
    let fill = Style::default().bg(FILL).fg(TEXT);
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].reset();
            buf[(x, y)].set_symbol(" ").set_style(fill);
        }
        buf[(r.x, y)]
            .set_symbol("▌")
            .set_style(Style::default().fg(edge).bg(FILL));
    }
    for (i, (line, bold)) in lines.iter().enumerate() {
        let y = r.y + 1 + u16::try_from(i).unwrap_or(0);
        let style = if *bold {
            fill.add_modifier(Modifier::BOLD)
        } else {
            fill
        };
        buf.set_stringn(r.x + 2, y, line, usize::from(r.width - 3), style);
    }
}

#[cfg(test)]
mod tests;

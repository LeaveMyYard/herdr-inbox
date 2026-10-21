//! Python `herdr.composer_state`.

use inbox_core::deliver::Composer;

use super::ansi::{strip_other, strip_sgr, undimmed};
use super::strip;

/// Claude Code's input-box prompt.
const PROMPT: char = '❯';

/// Python `_BORDER.match`: a run of at least three `─`, optionally with a
/// session tag before its last dashes (`───…─── node ─`).
fn is_border(s: &str) -> bool {
    let dashes = s.chars().take_while(|c| *c == '─').count();
    if dashes < 3 {
        return false;
    }
    let rest: Vec<char> = s.chars().skip(dashes).collect();
    if rest.is_empty() {
        return true;
    }
    // " " [^─\s] [^─]{0,40} " " ─+ $
    if rest.len() < 4 || rest[0] != ' ' || rest[1] == '─' || crate::text::py_isspace(rest[1]) {
        return false;
    }
    let tail = &rest[2..];
    let run = tail.iter().rev().take_while(|c| **c == '─').count();
    if run == 0 || tail.len() < run + 1 || tail[tail.len() - run - 1] != ' ' {
        return false;
    }
    let middle = &tail[..tail.len() - run - 1];
    middle.len() <= 40 && !middle.contains(&'─')
}

/// Python `_PLACEHOLDER.match`: `Try "…"`, the whole line.
fn is_placeholder(s: &str) -> bool {
    s.chars().count() >= 6 && s.starts_with("Try \"") && s.ends_with('"') && !s.contains('\n')
}

fn unbroken(s: &str) -> String {
    s.replace('\u{a0}', " ")
}

/// Claude Code's input box on `screen` (`pane read`, ANSI): the lines
/// between the last two `───` borders, the first starting with `❯`. Empty
/// only when recognised positively (a blank `❯` line, or exactly the dim
/// placeholder with at most one lit cell, and no other text in the box);
/// anything else in it is a draft; no box recognised is unknown.
pub fn composer_state(screen: &str) -> Composer {
    if screen.is_empty() {
        return Composer::Unknown;
    }
    let lines: Vec<&str> = screen.split('\n').collect();
    let plain: Vec<String> = lines.iter().map(|l| strip_sgr(&strip_other(l))).collect();
    let borders: Vec<usize> = (0..plain.len())
        .filter(|&i| is_border(strip(&plain[i])))
        .collect();
    let [.., top, bottom] = borders[..] else {
        return Composer::Unknown;
    };
    if bottom - top < 2 {
        return Composer::Unknown;
    }
    let first = unbroken(&plain[top + 1]);
    let first = strip(&first);
    let Some(content) = first.strip_prefix(PROMPT) else {
        return Composer::Unknown;
    };
    if (top + 2..bottom).any(|i| !strip(&unbroken(&plain[i])).is_empty()) {
        return Composer::Draft; // a second line: wrapped or multi-line text
    }
    let content = strip(content);
    if content.is_empty() {
        return Composer::Empty;
    }
    let lit = unbroken(&undimmed(lines[top + 1]));
    let lit = lit
        .split_once(PROMPT)
        .map_or(lit.as_str(), |(_, after)| after);
    if is_placeholder(content) && strip(lit).chars().count() <= 1 {
        return Composer::Empty; // the dim placeholder (a drawn cursor may light one cell)
    }
    Composer::Draft
}

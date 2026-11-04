//! A golden screen (rows of `[text, fg, bg, bold]` runs, `tests/fixtures/
//! screens/*.json`) against a rendered buffer, cell by cell: the char, the
//! fg and bg (`#rrggbb` or null for the terminal's default) and bold.

use ratatui_core::buffer::Buffer;
use ratatui_core::style::{Color, Modifier};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

/// A cell as the golden spells it. A wide char's second cell is `""`.
pub type Shown = (String, Option<String>, Option<String>, bool);

fn hex(c: Color) -> Option<String> {
    match c {
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

/// The golden rows as cells.
pub fn want(rows: &Value) -> Vec<Vec<Shown>> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let mut cells = Vec::new();
            for run in row.as_array().unwrap() {
                let fg = run[1].as_str().map(str::to_owned);
                let bg = run[2].as_str().map(str::to_owned);
                let bold = run[3].as_bool().unwrap();
                for ch in run[0].as_str().unwrap().chars() {
                    let w = ch.width().unwrap_or(0);
                    if w == 0 {
                        continue;
                    }
                    cells.push((ch.to_string(), fg.clone(), bg.clone(), bold));
                    for _ in 1..w {
                        cells.push((String::new(), fg.clone(), bg.clone(), bold));
                    }
                }
            }
            cells
        })
        .collect()
}

/// The buffer's rows as cells.
pub fn got(buf: &Buffer) -> Vec<Vec<Shown>> {
    let a = buf.area;
    (a.top()..a.bottom())
        .map(|y| {
            (a.left()..a.right())
                .map(|x| {
                    let c = &buf[(x, y)];
                    let bold = c.modifier.contains(Modifier::BOLD);
                    (c.symbol().to_string(), hex(c.fg), hex(c.bg), bold)
                })
                .collect()
        })
        .collect()
}

fn text(row: &[Shown]) -> String {
    row.iter().map(|c| c.0.as_str()).collect()
}

/// The differences between two screens: none, or the first few.
pub fn diff(name: &str, want: &[Vec<Shown>], got: &[Vec<Shown>]) -> Option<String> {
    if want.len() != got.len() {
        return Some(format!("{name}: {} rows, want {}", got.len(), want.len()));
    }
    let mut diffs = vec![];
    for (y, (a, b)) in want.iter().zip(got).enumerate() {
        if a.len() != b.len() {
            diffs.push(format!(
                "row {y}: width {} want {}\n  want {:?}\n  got  {:?}",
                b.len(),
                a.len(),
                text(a),
                text(b)
            ));
            continue;
        }
        for (x, (ca, cb)) in a.iter().zip(b).enumerate() {
            if ca != cb {
                diffs.push(format!(
                    "({x},{y}) want {ca:?} got {cb:?}\n  want {:?}\n  got  {:?}",
                    text(a),
                    text(b)
                ));
                break;
            }
        }
    }
    (!diffs.is_empty()).then(|| {
        format!(
            "{name}: {} rows differ, first:\n{}",
            diffs.len(),
            diffs[..diffs.len().min(4)].join("\n")
        )
    })
}

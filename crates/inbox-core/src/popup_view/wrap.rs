//! Textual's word wrap (`textual._wrap.compute_wrap_offsets` with
//! `fold=True`, then `Content._wrap_and_format`): a line breaks before the
//! chunk (a word with its trailing spaces) that does not fit, a chunk wider
//! than the line folds, every line but the last loses its trailing
//! whitespace, and each is cropped to the width.

use super::text::{Line, Span, char_width, py_space, width as str_width};

/// Textual's tab stops.
const TAB_SIZE: usize = 8;

/// `line` as the lines a `width`-cell widget shows.
pub(crate) fn wrap(line: &Line, width: usize) -> Vec<Line> {
    let line = expand_tabs(line);
    let plain: Vec<char> = line.iter().flat_map(|s| s.text.chars()).collect();
    let breaks = offsets(&plain, width);
    let mut cuts = vec![0];
    cuts.extend(breaks);
    cuts.push(plain.len());
    let pieces: Vec<Line> = cuts.windows(2).map(|w| slice(&line, w[0], w[1])).collect();
    let last = pieces.len() - 1;
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, piece)| {
            let piece = if i == last { piece } else { rstrip(piece) };
            truncate(piece, width)
        })
        .collect()
}

/// `compute_wrap_offsets`: the char indices to break `text` at.
pub(crate) fn offsets(text: &[char], width: usize) -> Vec<usize> {
    let mut breaks = Vec::new();
    if width == 0 {
        return breaks;
    }
    let mut cell_offset = 0;
    let mut end = 0;
    while end < text.len() {
        let mut start = end;
        if !py_space(text[end]) {
            while end < text.len() && !py_space(text[end]) {
                end += 1;
            }
        }
        while end < text.len() && py_space(text[end]) {
            end += 1;
        }
        let chunk = &text[start..end];
        let chunk_width: usize = chunk.iter().map(|&c| char_width(c)).sum();
        if width >= cell_offset + chunk_width {
            cell_offset += chunk_width;
        } else if chunk_width > width {
            let folded = fold(chunk, width);
            let n = folded.len();
            for (i, piece) in folded.into_iter().enumerate() {
                if start != 0 {
                    breaks.push(start);
                }
                if i + 1 == n {
                    cell_offset = piece.iter().map(|&c| char_width(c)).sum();
                } else {
                    start += piece.len();
                }
            }
        } else if cell_offset != 0 && start != 0 {
            breaks.push(start);
            cell_offset = chunk_width;
        }
    }
    breaks
}

/// A chunk too wide for a line, in pieces that fit (Textual's fold).
fn fold(chunk: &[char], width: usize) -> Vec<Vec<char>> {
    let mut lines: Vec<Vec<char>> = vec![Vec::new()];
    let mut total = 0;
    for &c in chunk {
        let w = char_width(c);
        if total + w > width {
            lines.push(vec![c]);
            total = w;
        } else {
            lines.last_mut().expect("one line").push(c);
            total += w;
        }
    }
    lines
}

/// The chars `from..to` of a line, spans kept.
fn slice(line: &Line, from: usize, to: usize) -> Line {
    let mut out = Vec::new();
    let mut at = 0;
    for span in line {
        let n = span.text.chars().count();
        let (a, b) = (from.max(at), to.min(at + n));
        if a < b {
            let text: String = span.text.chars().skip(a - at).take(b - a).collect();
            out.push(Span {
                text,
                look: span.look,
            });
        }
        at += n;
    }
    out
}

/// Python's `str.rstrip()` over the spans.
fn rstrip(mut line: Line) -> Line {
    while let Some(span) = line.last_mut() {
        let kept = span.text.trim_end_matches(py_space).len();
        span.text.truncate(kept);
        if !span.text.is_empty() {
            break;
        }
        line.pop();
    }
    line
}

/// Cropped to `width` cells.
pub(crate) fn truncate(line: Line, width: usize) -> Line {
    let mut out = Vec::new();
    let mut used = 0;
    for span in line {
        let mut text = String::new();
        for c in span.text.chars() {
            let w = char_width(c);
            if used + w > width {
                break;
            }
            used += w;
            text.push(c);
        }
        let full = text.len() == span.text.len();
        if !text.is_empty() {
            out.push(Span {
                text,
                look: span.look,
            });
        }
        if !full {
            break;
        }
    }
    out
}

/// Tabs to spaces, to the next stop of [`TAB_SIZE`] (Textual's `expand_tabs`).
fn expand_tabs(line: &Line) -> Line {
    if !line.iter().any(|s| s.text.contains('\t')) {
        return line.clone();
    }
    let mut column = 0;
    line.iter()
        .map(|span| {
            let mut text = String::new();
            for c in span.text.chars() {
                if c == '\t' {
                    let n = TAB_SIZE - column % TAB_SIZE;
                    text.extend(std::iter::repeat_n(' ', n));
                    column += n;
                } else {
                    text.push(c);
                    column += str_width(c.encode_utf8(&mut [0; 4]));
                }
            }
            Span {
                text,
                look: span.look,
            }
        })
        .collect()
}

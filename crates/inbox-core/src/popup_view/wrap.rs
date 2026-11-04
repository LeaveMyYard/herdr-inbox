//! The word wrap of Textual's `Content` (`Content._wrap_and_format` over
//! `rich._wrap.divide_line` with `fold=True`): a line breaks before the
//! word that does not fit, a word wider than the line folds, every line but
//! the last loses its trailing whitespace, and each is cropped to the width.

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

/// `rich._wrap.divide_line` (what Textual's `Content` wraps with): the
/// char indices to break `text` at. A word is its leading and trailing
/// whitespace with it; it fits if it does without its trailing whitespace.
pub(crate) fn offsets(text: &[char], width: usize) -> Vec<usize> {
    let mut breaks = Vec::new();
    if width == 0 {
        return breaks;
    }
    let cells = |s: &[char]| s.iter().map(|&c| char_width(c)).sum::<usize>();
    let mut cell_offset = 0;
    let mut end = 0;
    while end < text.len() {
        let mut start = end;
        while end < text.len() && py_space(text[end]) {
            end += 1;
        }
        if end == text.len() {
            break; // trailing whitespace alone is no word (`\s*\S+\s*`)
        }
        while end < text.len() && !py_space(text[end]) {
            end += 1;
        }
        let bare = end;
        while end < text.len() && py_space(text[end]) {
            end += 1;
        }
        let word = &text[start..end];
        let word_length = cells(&text[start..bare]);
        if width as i64 - cell_offset as i64 >= word_length as i64 {
            cell_offset += cells(word);
        } else if word_length > width {
            let folded = chop(word, width);
            let n = folded.len();
            for (i, piece) in folded.into_iter().enumerate() {
                if start != 0 {
                    breaks.push(start);
                }
                if i + 1 == n {
                    cell_offset = cells(&piece);
                } else {
                    start += piece.len();
                }
            }
        } else if cell_offset != 0 && start != 0 {
            breaks.push(start);
            cell_offset = cells(word);
        }
    }
    breaks
}

/// `rich.cells.chop_cells`: `word` in pieces of at most `width` cells.
fn chop(word: &[char], width: usize) -> Vec<Vec<char>> {
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut size = 0;
    for &c in word {
        let w = char_width(c);
        if size + w > width {
            lines.push(std::mem::take(&mut line));
            size = 0;
        }
        size += w;
        line.push(c);
    }
    if size > 0 {
        lines.push(line);
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

/// Cropped to `width` cells; a wide char cut in half becomes blanks
/// (rich's `set_cell_size`).
pub(crate) fn truncate(line: Line, width: usize) -> Line {
    let mut out = Vec::new();
    let mut used = 0;
    for span in line {
        let mut text = String::new();
        let mut cut = false;
        for c in span.text.chars() {
            let w = char_width(c);
            if used + w > width {
                cut = true;
                break;
            }
            used += w;
            text.push(c);
        }
        if cut {
            text.extend(std::iter::repeat_n(' ', width - used));
        }
        if !text.is_empty() {
            out.push(Span {
                text,
                look: span.look,
            });
        }
        if cut {
            break;
        }
    }
    out
}

/// Tabs to spaces, to the next stop of [`TAB_SIZE`] cells (Textual's
/// `Content.expand_tabs` of styled text; unstyled text counts characters,
/// which differs only after a wide char).
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

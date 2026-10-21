//! The two escape patterns of Python `herdr.py`, as `re.sub` applies them.

const ESC: char = '\x1b';
const BEL: char = '\x07';

/// `_OTHER_ESC.sub("", s)`: `\x1b\[[0-9;?]*[A-Za-ln-z]`, `\x1b\][^\x07]*\x07`
/// and `\r` removed (SGR, which ends in `m`, is left for [`strip_sgr`]).
pub(super) fn strip_other(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (String::new(), 0);
    while i < c.len() {
        if let Some(end) = csi_other(&c, i).or_else(|| osc(&c, i)) {
            i = end;
        } else if c[i] == '\r' {
            i += 1;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// A CSI ending in a letter other than `m` at `i`: the index after it.
fn csi_other(c: &[char], i: usize) -> Option<usize> {
    if c.get(i) != Some(&ESC) || c.get(i + 1) != Some(&'[') {
        return None;
    }
    let mut j = i + 2;
    while c
        .get(j)
        .is_some_and(|ch| ch.is_ascii_digit() || *ch == ';' || *ch == '?')
    {
        j += 1;
    }
    c.get(j)
        .filter(|ch| ch.is_ascii_alphabetic() && **ch != 'm')
        .map(|_| j + 1)
}

/// An OSC up to its BEL at `i`: the index after it.
fn osc(c: &[char], i: usize) -> Option<usize> {
    if c.get(i) != Some(&ESC) || c.get(i + 1) != Some(&']') {
        return None;
    }
    c[i + 2..]
        .iter()
        .position(|ch| *ch == BEL)
        .map(|k| i + 2 + k + 1)
}

/// The SGR at `i` (`\x1b\[([0-9;]*)m`): its parameters and the index after it.
pub(super) fn sgr(c: &[char], i: usize) -> Option<(String, usize)> {
    if c.get(i) != Some(&ESC) || c.get(i + 1) != Some(&'[') {
        return None;
    }
    let mut j = i + 2;
    while c.get(j).is_some_and(|ch| ch.is_ascii_digit() || *ch == ';') {
        j += 1;
    }
    (c.get(j) == Some(&'m')).then(|| (c[i + 2..j].iter().collect(), j + 1))
}

/// `_SGR.sub("", s)`.
pub(super) fn strip_sgr(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (String::new(), 0);
    while i < c.len() {
        match sgr(&c, i) {
            Some((_, end)) => i = end,
            None => {
                out.push(c[i]);
                i += 1;
            }
        }
    }
    out
}

/// Python `_undimmed`: the characters of an ANSI line that are not dim
/// (SGR 2; 0, an empty code and 22 end it).
pub(super) fn undimmed(line: &str) -> String {
    let c: Vec<char> = strip_other(line).chars().collect();
    let (mut out, mut dim, mut i) = (String::new(), false, 0);
    while i < c.len() {
        match sgr(&c, i) {
            Some((params, end)) => {
                let params = if params.is_empty() {
                    "0".to_string()
                } else {
                    params
                };
                for code in params.split(';') {
                    match code {
                        "0" | "" | "22" => dim = false,
                        "2" => dim = true,
                        _ => {}
                    }
                }
                i = end;
            }
            None => {
                if !dim {
                    out.push(c[i]);
                }
                i += 1;
            }
        }
    }
    out
}

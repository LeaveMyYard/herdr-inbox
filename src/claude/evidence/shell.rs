//! Which questions a shell command ANSWERS with the inbox CLI (Python
//! `transcript.own_answer_qids`, `own_answer_succeeded`; review #44 r3).
//! Only an actual invocation at the start of a command segment counts:
//! `[path/]inbox answer [Q]<n>`, `$VAR answer <n>` with `VAR` set to an
//! inbox path earlier in the command, `uv run [flags] inbox answer <n>`,
//! `python[3] -m herdr_inbox.cli answer <n>`. A path, a repo name, a grep
//! pattern or a comment body never does; neither does anything after the
//! command points the CLI at another database.
//!
//! **SAFETY**: a question counted here is marked seen and never typed again.

use std::collections::BTreeSet;

use super::shlex::split;
use crate::cli::args::pyint;
use crate::store::norm::is_word;
use crate::text::py_isspace;

const SEPARATORS: &str = ";&|\n";
/// `uv run` flags that take a value (Python `_VALUE_FLAGS`).
const VALUE_FLAGS: [&str; 6] = [
    "--project",
    "--directory",
    "--python",
    "--with",
    "--from",
    "-p",
];

/// Python `_ASSIGN.match(token)` (`^([A-Za-z_]\w*)=(\S+)$`): `(name, value)`.
fn assign(token: &str) -> Option<(&str, &str)> {
    // `$` also matches before one final newline
    let t = token.strip_suffix('\n').unwrap_or(token);
    let (name, value) = t.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let ok_name = (first.is_ascii_alphabetic() || first == '_') && chars.all(is_word);
    let ok_value = !value.is_empty() && !value.chars().any(py_isspace);
    (ok_name && ok_value).then_some((name, value))
}

/// Python `_QID.match` (`^Q?(\d+)$`) read with `int()`.
fn qid(token: &str) -> Option<i64> {
    let t = token.strip_suffix('\n').unwrap_or(token);
    let digits = t.strip_prefix('Q').unwrap_or(t);
    let all_digits = !digits.is_empty()
        && digits
            .chars()
            .all(|c| pyint::parse(&c.to_string()).is_some());
    all_digits.then(|| pyint::parse(digits)).flatten()
}

fn is_inbox(token: &str) -> bool {
    token == "inbox" || token.ends_with("/inbox")
}

/// The question ids `command` answers (empty when shlex refuses it).
pub fn own_answer_qids(command: &str) -> BTreeSet<i64> {
    let Ok(all) = split(command) else {
        return BTreeSet::new();
    };
    let mut segments: Vec<Vec<String>> = vec![Vec::new()];
    for tok in all {
        if !tok.is_empty() && tok.chars().all(|c| SEPARATORS.contains(c)) {
            segments.push(Vec::new());
        } else {
            segments.last_mut().expect("one segment").push(tok);
        }
    }
    let (mut inbox_vars, mut out, mut elsewhere) = (BTreeSet::new(), BTreeSet::new(), false);
    for seg in segments {
        let mut tokens: Vec<String> = seg;
        if let Some(first) = tokens.first_mut() {
            *first = first.trim_start_matches('\n').to_string(); // "&&\n$B" glues
        }
        tokens.retain(|t| !t.is_empty());
        // a CLI pointed at another database answers THAT database's question:
        // from such a setting on, nothing counts (final pass #44 L1)
        if tokens
            .iter()
            .filter_map(|t| assign(t))
            .any(|(n, _)| n == "HERDR_INBOX_DB" || n == "XDG_STATE_HOME")
        {
            elsewhere = true;
        }
        if elsewhere {
            continue;
        }
        if let [only] = tokens.as_slice()
            && let Some((name, value)) = assign(only)
        {
            if is_inbox(value) {
                inbox_vars.insert(name.to_string());
            }
            continue;
        }
        let start = tokens.iter().take_while(|t| assign(t).is_some()).count();
        let tokens = &tokens[start..];
        let Some(head) = tokens.first() else {
            continue;
        };
        let var_inbox =
            head.starts_with('$') && inbox_vars.contains(head.trim_matches(['$', '{', '}']));
        let rest: Option<&[String]> = if is_inbox(head) || var_inbox {
            Some(&tokens[1..])
        } else if head == "uv" && tokens.len() > 2 && tokens[1] == "run" {
            let mut i = 2;
            while i < tokens.len() && tokens[i].starts_with('-') {
                i += if VALUE_FLAGS.contains(&tokens[i].as_str()) {
                    2
                } else {
                    1
                };
            }
            (i < tokens.len() && is_inbox(&tokens[i])).then(|| &tokens[i + 1..])
        } else if matches!(head.rsplit('/').next(), Some("python" | "python3"))
            && tokens
                .get(1..3)
                .is_some_and(|t| t[0] == "-m" && t[1] == "herdr_inbox.cli")
        {
            Some(&tokens[3..])
        } else {
            None
        };
        if let Some(rest) = rest
            && rest.len() >= 2
            && rest[0] == "answer"
            && let Some(n) = qid(&rest[1])
        {
            out.insert(n);
        }
    }
    out
}

/// The CLI's own success line for `qid` starts a line of `output`, and it
/// did not say the question was not open (Python `own_answer_succeeded`).
pub fn own_answer_succeeded(output: &str, qid: i64) -> bool {
    let lines: Vec<&str> = py_lines(output)
        .map(|l| l.trim_matches(py_isspace))
        .collect();
    if lines
        .iter()
        .any(|l| l.starts_with(&format!("Q{qid} is not open")))
    {
        return false;
    }
    let (a, b) = (
        format!("Q{qid} answered, "),
        format!("Q{qid} answered by its own conversation"),
    );
    lines.iter().any(|l| l.starts_with(&a) || l.starts_with(&b))
}

/// Python's `str.splitlines()` boundaries.
fn py_lines(text: &str) -> impl Iterator<Item = &str> {
    let breaks = |c: char| {
        matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        )
    };
    // "\r\n" is one break: the empty piece between them is a line here, but
    // an empty line can never start with a success line, so it is harmless
    text.split(breaks)
}

#[cfg(test)]
mod tests;

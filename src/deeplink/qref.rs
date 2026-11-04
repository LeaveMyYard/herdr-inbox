//! Question references as `inbox` commands and deep links take them (Python
//! `deeplink.question_ref`, `question_id`).

use crate::pyint;

/// A local question reference (`Q12`, `q12`, `12`) as its id (Python
/// `deeplink.question_id`): at most one `Q`/`q` right before the number,
/// after an optional `machine:` that makes it no local id. `None` where
/// Python raises `ValueError` (or an id SQLite cannot hold).
pub fn question_id(value: &str) -> Option<i64> {
    pyint::parse(&question_ref(value))
}

/// `[machine:]Qn` without its Q (Python `deeplink.question_ref`, #55):
/// `qnap:Q12` is `qnap:12`, `Q12` and `q12` are `12`, `QQ12` is `Q12`.
pub fn question_ref(value: &str) -> String {
    let (machine, refpart) = match value.rfind(':') {
        Some(i) => (&value[..=i], &value[i + 1..]),
        None => ("", value),
    };
    let refpart = refpart
        .strip_prefix('Q')
        .or_else(|| refpart.strip_prefix('q'))
        .unwrap_or(refpart);
    format!("{machine}{refpart}")
}

#[cfg(test)]
mod tests;

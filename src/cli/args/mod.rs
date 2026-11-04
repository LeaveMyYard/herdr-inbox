//! The CLI's arguments, parsed as Python's `argparse` parses `inbox`'s
//! (`cli.main`): the same accepted spellings, the same help, and the same
//! usage errors on stderr with exit code 2.
//!
//! - [`parse`]: one subcommand's options and positionals, in argparse's
//!   order (an error is reported where argparse meets it).
//! - [`pyint`]: Python's `int()` of a string, for question ids.
//! - [`pyrepr`]: Python's `repr()` of a string, as the errors quote values.

pub mod parse;
pub mod pyint;
pub mod pyrepr;

/// `inbox`'s own usage, as argparse renders it without a terminal (80
/// columns): every usage error of a subcommand's extra arguments prints it.
pub const TOP_USAGE: &str = "usage: inbox [-h]\n             {mcp,tui,open,list,answer,dismiss,redeliver,sync-tokens,radar-sidebar,clean-venvs,deliver,verify,worker,export,apply,remote,hook} ...\n";

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

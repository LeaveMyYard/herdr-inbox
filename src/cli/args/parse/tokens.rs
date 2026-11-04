//! How argparse reads each word: after the first `--` every word is a
//! positional; `-h`/`--help`, the options and their unambiguous prefixes
//! (with an optional `=value`) are options; a negative number, a lone `-`
//! and a word with a space are positionals; any other `-word` is an unknown
//! option.

use unicode_general_category::{GeneralCategory, get_general_category};

use super::{Command, Opt};

/// The help option's name in errors.
pub const HELP: &str = "-h/--help";

/// One word, as argparse reads it.
pub enum Token<'a> {
    /// A positional (or an option's value).
    Word(&'a str),
    /// `-h`/`--help`, with a value given after `=`.
    Help(Option<&'a str>),
    /// An option, with a value given after `=`.
    Opt(&'static Opt, Option<&'a str>),
    /// An option nobody declared.
    Unknown(&'a str),
}

impl<'a> Token<'a> {
    /// The word, if it is one.
    pub fn word(&self) -> Option<&'a str> {
        match self {
            Token::Word(w) => Some(w),
            _ => None,
        }
    }
}

/// Every word of `argv`, read.
pub fn classify<'a>(cmd: &Command, argv: &'a [String]) -> Vec<Token<'a>> {
    let mut out = Vec::new();
    let mut options_end = false;
    for w in argv {
        let w = w.as_str();
        if options_end || !w.starts_with('-') || w == "-" {
            out.push(Token::Word(w));
        } else if w == "--" {
            options_end = true;
        } else {
            out.push(option(cmd, w));
        }
    }
    out
}

fn option<'a>(cmd: &Command, w: &'a str) -> Token<'a> {
    // `-hx` is `-h` then `-x`: the help comes first
    if w == "--help" || (w.starts_with("-h") && !w.starts_with("--")) {
        return Token::Help(None);
    }
    if let Some(opt) = cmd.options.iter().find(|o| o.name == w) {
        return Token::Opt(opt, None);
    }
    if w.starts_with("--") {
        let (name, value) = match w.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (w, None),
        };
        let mut hits = cmd.options.iter().filter(|o| o.name.starts_with(name));
        let help = "--help".starts_with(name);
        match (hits.next(), hits.next(), help) {
            (None, _, true) => return Token::Help(value),
            (Some(opt), None, false) => return Token::Opt(opt, value),
            _ => {}
        }
    }
    if is_negative_number(w) || w.contains(' ') {
        return Token::Word(w);
    }
    Token::Unknown(w)
}

/// argparse's `^-\d+$|^-\d*\.\d+$` (`\d`: any Unicode decimal digit).
fn is_negative_number(w: &str) -> bool {
    let digits = |s: &str| s.chars().all(is_nd);
    let Some(rest) = w.strip_prefix('-') else {
        return false;
    };
    match rest.split_once('.') {
        None => !rest.is_empty() && digits(rest),
        Some((a, b)) => digits(a) && !b.is_empty() && digits(b),
    }
}

fn is_nd(c: char) -> bool {
    get_general_category(c) == GeneralCategory::DecimalNumber
}

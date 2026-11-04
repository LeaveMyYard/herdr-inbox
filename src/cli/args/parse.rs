//! One subcommand's arguments, parsed the way `argparse` parses them:
//! positionals taken in runs between options, `--` ending the options, a
//! long option's unambiguous prefix accepted, `-h`/`--help` printing the
//! help at once, and every usage error worded and ordered as argparse does
//! (a value is checked when its run is taken; missing positionals after
//! the whole line; arguments nobody takes, last, by the top parser).

use super::TOP_USAGE;
use super::pyrepr::repr;

/// How many words a positional takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nargs {
    /// Exactly one.
    One,
    /// One or more (`nargs="+"`).
    Plus,
}

/// An argparse `type`: the name errors call it (`question id`), and the
/// check a value must pass.
pub type Check = (&'static str, fn(&str) -> bool);

/// A positional argument.
#[derive(Clone, Copy, Debug)]
pub struct Positional {
    /// Its name in errors (`id`).
    pub name: &'static str,
    /// How many words it takes.
    pub nargs: Nargs,
    /// Its argparse `type`: the name errors call it, and the check.
    pub check: Option<Check>,
}

/// A subcommand.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// `inbox answer`.
    pub prog: &'static str,
    /// Its usage line, `usage: inbox answer [-h] id text [text ...]`.
    pub usage: &'static str,
    /// Its whole `--help` text.
    pub help: &'static str,
    /// Its flags besides `--help` (`--all`).
    pub flags: &'static [&'static str],
    /// Its positionals, in order.
    pub positionals: &'static [Positional],
}

/// What a command line held.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The flags given.
    pub flags: Vec<&'static str>,
    /// Each positional's words, in [`Command::positionals`]' order.
    pub values: Vec<Vec<String>>,
}

impl Args {
    /// The flag was given.
    pub fn has(&self, flag: &str) -> bool {
        self.flags.contains(&flag)
    }
}

/// The parse's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Run the command with these.
    Run(Args),
    /// Print and exit (help: 0 on stdout; a usage error: 2 on stderr).
    Exit {
        /// The exit code.
        code: i32,
        /// What goes to stdout.
        stdout: String,
        /// What goes to stderr.
        stderr: String,
    },
}

enum Token<'a> {
    Word(&'a str),
    Help,
    Flag(&'static str),
    Unknown(&'a str),
    /// `--all=x`: an option that takes no value, given one (its name in errors, the value).
    Explicit(&'static str, &'a str),
}

/// Parses `argv` (the words after the subcommand) for `cmd`.
pub fn parse(cmd: &Command, argv: &[String]) -> Parsed {
    let tokens = classify(cmd, argv);
    let mut args = Args {
        values: vec![Vec::new(); cmd.positionals.len()],
        ..Args::default()
    };
    let mut next = 0; // the first positional not yet taken
    let mut extras: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let run_end = (i..tokens.len())
            .find(|&j| !matches!(tokens[j], Token::Word(_)))
            .unwrap_or(tokens.len());
        let words: Vec<&str> = tokens[i..run_end]
            .iter()
            .filter_map(|t| match t {
                Token::Word(w) => Some(*w),
                _ => None,
            })
            .collect();
        let mut taken = 0;
        while next < cmd.positionals.len() && taken < words.len() {
            let p = cmd.positionals[next];
            let n = match p.nargs {
                Nargs::One => 1,
                Nargs::Plus => words.len() - taken,
            };
            for w in &words[taken..taken + n] {
                if let Some((kind, ok)) = p.check
                    && !ok(w)
                {
                    let msg = format!("argument {}: invalid {kind} value: {}", p.name, repr(w));
                    return error(cmd.usage, cmd.prog, &msg);
                }
            }
            args.values[next] = words[taken..taken + n]
                .iter()
                .map(|w| w.to_string())
                .collect();
            taken += n;
            next += 1;
        }
        extras.extend(&words[taken..]);
        i = run_end;
        match tokens.get(i) {
            Some(Token::Help) => {
                return Parsed::Exit {
                    code: 0,
                    stdout: cmd.help.to_string(),
                    stderr: String::new(),
                };
            }
            Some(Token::Flag(f)) => args.flags.push(f),
            Some(Token::Unknown(u)) => extras.push(u),
            Some(Token::Explicit(name, value)) => {
                let msg = format!("argument {name}: ignored explicit argument {}", repr(value));
                return error(cmd.usage, cmd.prog, &msg);
            }
            _ => {}
        }
        i += 1;
    }
    let missing: Vec<&str> = cmd.positionals[next..].iter().map(|p| p.name).collect();
    if !missing.is_empty() {
        let msg = format!(
            "the following arguments are required: {}",
            missing.join(", ")
        );
        return error(cmd.usage, cmd.prog, &msg);
    }
    if !extras.is_empty() {
        let msg = format!("unrecognized arguments: {}", extras.join(" "));
        return error(TOP_USAGE.trim_end(), "inbox", &msg);
    }
    Parsed::Run(args)
}

fn error(usage: &str, prog: &str, msg: &str) -> Parsed {
    Parsed::Exit {
        code: 2,
        stdout: String::new(),
        stderr: format!("{usage}\n{prog}: error: {msg}\n"),
    }
}

/// argparse's reading of each word: after the first `--` every word is a
/// positional; `-h`/`--help`, the flags and their unambiguous prefixes are
/// options; a negative number, a lone `-` and a word with a space are
/// positionals; any other `-word` is an unknown option.
fn classify<'a>(cmd: &Command, argv: &'a [String]) -> Vec<Token<'a>> {
    let mut out = Vec::new();
    let mut options_end = false;
    for w in argv {
        let w = w.as_str();
        if options_end || !w.starts_with('-') || w == "-" {
            out.push(Token::Word(w));
            continue;
        }
        if w == "--" {
            options_end = true;
            continue;
        }
        out.push(option(cmd, w));
    }
    out
}

/// The help option's name in errors.
const HELP: &str = "-h/--help";

fn option<'a>(cmd: &Command, w: &'a str) -> Token<'a> {
    // `-hx` is `-h` then `-x`: the help comes first
    if w == "--help" || (w.starts_with("-h") && !w.starts_with("--")) {
        return Token::Help;
    }
    if let Some(f) = cmd.flags.iter().find(|f| **f == w) {
        return Token::Flag(f);
    }
    if w.starts_with("--") {
        let (name, value) = match w.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (w, None),
        };
        let mut hits = cmd.flags.iter().copied().filter(|f| f.starts_with(name));
        let help = "--help".starts_with(name);
        let hit = match (hits.next(), hits.next(), help) {
            (None, _, true) => Some(None),
            (Some(f), None, false) => Some(Some(f)),
            _ => None,
        };
        match (hit, value) {
            (Some(None), None) => return Token::Help,
            (Some(Some(f)), None) => return Token::Flag(f),
            (Some(f), Some(v)) => return Token::Explicit(f.unwrap_or(HELP), v),
            (None, _) => {}
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
    use unicode_general_category::{GeneralCategory, get_general_category};
    get_general_category(c) == GeneralCategory::DecimalNumber
}

#[cfg(test)]
mod tests;

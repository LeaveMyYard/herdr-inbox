//! One subcommand's arguments, parsed the way `argparse` parses them:
//! positionals taken in runs between options, `--` ending the options, a
//! long option's unambiguous prefix accepted, an option's value as the next
//! word or after `=`, mutually exclusive options, `-h`/`--help` printing the
//! help at once, and every usage error worded and ordered as argparse does
//! (a value is checked when its run is taken; missing positionals after the
//! whole line; arguments nobody takes, last, by the top parser).

mod tokens;

use super::TOP_USAGE;
use super::pyrepr::repr;
use tokens::{HELP, Token, classify};

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
    /// Its argparse `type`, if any.
    pub check: Option<Check>,
}

/// An option besides `--help`.
#[derive(Clone, Copy, Debug)]
pub struct Opt {
    /// `--all`.
    pub name: &'static str,
    /// It takes one value (`--question Qn`); else it is a flag.
    pub value: bool,
    /// Its mutually exclusive group, if in one.
    pub group: Option<u8>,
}

impl Opt {
    /// A flag.
    pub const fn flag(name: &'static str) -> Opt {
        Opt {
            name,
            value: false,
            group: None,
        }
    }
}

/// A subcommand.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// `inbox answer`.
    pub prog: &'static str,
    /// Its usage, `usage: inbox answer [-h] id text [text ...]` (wrapped as
    /// argparse wraps it).
    pub usage: &'static str,
    /// Its whole `--help` text.
    pub help: &'static str,
    /// Its options besides `--help`.
    pub options: &'static [Opt],
    /// Its positionals, in order.
    pub positionals: &'static [Positional],
}

/// What a command line held.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The options given, in order, with their values.
    pub given: Vec<(&'static str, Option<String>)>,
    /// Each positional's words, in [`Command::positionals`]' order.
    pub values: Vec<Vec<String>>,
}

impl Args {
    /// The option was given.
    pub fn has(&self, name: &str) -> bool {
        self.given.iter().any(|(n, _)| *n == name)
    }

    /// The option's value, the last one given.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.given
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .and_then(|(_, v)| v.as_deref())
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
        let words: Vec<&str> = tokens[i..run_end].iter().filter_map(Token::word).collect();
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
        let Some(token) = tokens.get(i) else { break };
        i += 1;
        let (opt, value) = match token {
            Token::Help(None) => {
                return Parsed::Exit {
                    code: 0,
                    stdout: cmd.help.to_string(),
                    stderr: String::new(),
                };
            }
            Token::Help(Some(v)) => return ignored(cmd, HELP, v),
            Token::Unknown(u) => {
                extras.push(u);
                continue;
            }
            Token::Opt(opt, Some(v)) if !opt.value => return ignored(cmd, opt.name, v),
            Token::Opt(opt, Some(v)) => (*opt, Some(v.to_string())),
            Token::Opt(opt, None) if !opt.value => (*opt, None),
            Token::Opt(opt, None) => match tokens.get(i).and_then(Token::word) {
                Some(v) => {
                    i += 1;
                    (*opt, Some(v.to_string()))
                }
                None => {
                    let msg = format!("argument {}: expected one argument", opt.name);
                    return error(cmd.usage, cmd.prog, &msg);
                }
            },
            Token::Word(_) => unreachable!("runs of words are taken above"),
        };
        if let Some(group) = opt.group {
            let rival = args.given.iter().find(|(name, _)| {
                *name != opt.name
                    && cmd
                        .options
                        .iter()
                        .any(|o| o.name == *name && o.group == Some(group))
            });
            if let Some((rival, _)) = rival {
                let msg = format!("argument {}: not allowed with argument {rival}", opt.name);
                return error(cmd.usage, cmd.prog, &msg);
            }
        }
        args.given.push((opt.name, value));
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

/// `inbox`'s own `parser.error(msg)`: the top usage, exit 2.
pub fn top_error(msg: &str) -> Parsed {
    error(TOP_USAGE.trim_end(), "inbox", msg)
}

fn ignored(cmd: &Command, name: &str, value: &str) -> Parsed {
    let msg = format!("argument {name}: ignored explicit argument {}", repr(value));
    error(cmd.usage, cmd.prog, &msg)
}

fn error(usage: &str, prog: &str, msg: &str) -> Parsed {
    Parsed::Exit {
        code: 2,
        stdout: String::new(),
        stderr: format!("{usage}\n{prog}: error: {msg}\n"),
    }
}

#[cfg(test)]
mod tests;

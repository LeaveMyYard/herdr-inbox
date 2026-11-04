//! The subcommands' arguments and help, as `cli.main` declares them to
//! argparse (and as argparse renders their help).

use super::args::parse::{Check, Command, Nargs, Positional};
use super::args::question_id;

/// argparse `type=_question_id`, named `question id` in errors.
const QUESTION_ID: Option<Check> = Some(("question id", |v| question_id(v).is_some()));

const ID: Positional = Positional {
    name: "id",
    nargs: Nargs::One,
    check: QUESTION_ID,
};

/// `inbox list [--all]`.
pub const LIST: Command = Command {
    prog: "inbox list",
    usage: "usage: inbox list [-h] [--all]",
    help: "usage: inbox list [-h] [--all]\n\noptions:\n  -h, --help  show this help message and exit\n  \
           --all       include answered and closed questions\n",
    flags: &["--all"],
    positionals: &[],
};

/// `inbox answer ID TEXT...`.
pub const ANSWER: Command = Command {
    prog: "inbox answer",
    usage: "usage: inbox answer [-h] id text [text ...]",
    help: "usage: inbox answer [-h] id text [text ...]\n\npositional arguments:\n  id\n  text\n\n\
           options:\n  -h, --help  show this help message and exit\n",
    flags: &[],
    positionals: &[
        ID,
        Positional {
            name: "text",
            nargs: Nargs::Plus,
            check: None,
        },
    ],
};

/// `inbox dismiss ID`.
pub const DISMISS: Command = Command {
    prog: "inbox dismiss",
    usage: "usage: inbox dismiss [-h] id",
    help: "usage: inbox dismiss [-h] id\n\npositional arguments:\n  id\n\noptions:\n  \
           -h, --help  show this help message and exit\n",
    flags: &[],
    positionals: &[ID],
};

/// The subcommand named `cmd`, if this CLI has it.
pub fn command(cmd: &str) -> Option<&'static Command> {
    match cmd {
        "list" => Some(&LIST),
        "answer" => Some(&ANSWER),
        "dismiss" => Some(&DISMISS),
        _ => None,
    }
}

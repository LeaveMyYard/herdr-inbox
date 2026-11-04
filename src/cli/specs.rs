//! The subcommands' arguments and help, as `cli.main` declares them to
//! argparse (and as argparse renders their help).

use super::args::parse::{Check, Command, Nargs, Opt, Positional};
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
    options: &[Opt::flag("--all")],
    positionals: &[],
};

/// `inbox answer ID TEXT...`.
pub const ANSWER: Command = Command {
    prog: "inbox answer",
    usage: "usage: inbox answer [-h] id text [text ...]",
    help: "usage: inbox answer [-h] id text [text ...]\n\npositional arguments:\n  id\n  text\n\n\
           options:\n  -h, --help  show this help message and exit\n",
    options: &[],
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
    options: &[],
    positionals: &[ID],
};

const fn target(name: &'static str) -> Opt {
    Opt {
        name,
        value: true,
        group: Some(0),
    }
}

/// `inbox open [--question Qn | --group G | --pane PANE_ID | --session SID] [--no-hold]`.
pub const OPEN: Command = Command {
    prog: "inbox open",
    usage: "usage: inbox open [-h] [--question Qn | --group G | --pane PANE_ID |\n                  \
            --session SID] [--no-hold]",
    help: "usage: inbox open [-h] [--question Qn | --group G | --pane PANE_ID |\n                  \
           --session SID] [--no-hold]\n\noptions:\n  -h, --help      show this help message and exit\n  \
           --question Qn   open on this question\n  --group G       open on this group\n  \
           --pane PANE_ID  open on this pane's oldest open group\n  \
           --session SID   open on this Claude session's oldest open group\n  \
           --no-hold       open like a normal popup (0.4 s arming, then Enter works)\n                  \
           instead of held until Tab/→; pass it only when an explicit\n                  \
           operator key or click opened the inbox\n",
    options: &[
        target("--question"),
        target("--group"),
        target("--pane"),
        target("--session"),
        Opt::flag("--no-hold"),
    ],
    positionals: &[],
};

/// `inbox sync-tokens [--remove-prefixes]`.
pub const SYNC_TOKENS: Command = Command {
    prog: "inbox sync-tokens",
    usage: "usage: inbox sync-tokens [-h] [--remove-prefixes]",
    help: "usage: inbox sync-tokens [-h] [--remove-prefixes]\n\noptions:\n  \
           -h, --help         show this help message and exit\n  \
           --remove-prefixes  strip every tab prefix (before uninstalling)\n",
    options: &[Opt::flag("--remove-prefixes")],
    positionals: &[],
};

/// `inbox radar-sidebar [--remove]`.
pub const RADAR_SIDEBAR: Command = Command {
    prog: "inbox radar-sidebar",
    usage: "usage: inbox radar-sidebar [-h] [--remove]",
    help: "usage: inbox radar-sidebar [-h] [--remove]\n\noptions:\n  \
           -h, --help  show this help message and exit\n  \
           --remove    take it out again (before uninstalling)\n",
    options: &[Opt::flag("--remove")],
    positionals: &[],
};

/// A subcommand without arguments: `inbox <name> [-h]`.
macro_rules! bare {
    ($name:literal) => {
        Command {
            prog: concat!("inbox ", $name),
            usage: concat!("usage: inbox ", $name, " [-h]"),
            help: concat!(
                "usage: inbox ",
                $name,
                " [-h]\n\noptions:\n  -h, --help  show this help message and exit\n"
            ),
            options: &[],
            positionals: &[],
        }
    };
}

/// `inbox export` (remote machines; the request on stdin).
pub const EXPORT: Command = bare!("export");

/// `inbox apply` (remote machines; the request on stdin).
pub const APPLY: Command = bare!("apply");

/// The subcommand named `cmd`, if this CLI has it.
pub fn command(cmd: &str) -> Option<&'static Command> {
    match cmd {
        "list" => Some(&LIST),
        "answer" => Some(&ANSWER),
        "dismiss" => Some(&DISMISS),
        "open" => Some(&OPEN),
        "sync-tokens" => Some(&SYNC_TOKENS),
        "radar-sidebar" => Some(&RADAR_SIDEBAR),
        "export" => Some(&EXPORT),
        "apply" => Some(&APPLY),
        _ => None,
    }
}

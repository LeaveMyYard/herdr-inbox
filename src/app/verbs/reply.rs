//! What a verb says: one JSON object on stdout and an exit code. Empty stdout
//! never means success, and nothing here echoes `--detail`.

use serde_json::{Value, json};

use super::apply::Done;
use super::args::Request;

/// The first line of `app --help`, which a mod's probe reads.
pub const HELP: &str = "herdr-inbox app v1\n\
usage: herdr-inbox app <verb> ... --json (docs/protocol/app-commands-v1.md)\n";

/// A refusal's code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// The token does not match, or the row no longer exists.
    Stale,
    /// `retry` on an answer a process holds a live claim on.
    AlreadySending,
    /// The detail or `--question` does not fit the row.
    Invalid,
    /// A remote row, or a verb the row's actions do not list.
    ReadOnly,
    /// The database stayed locked for the budget.
    Busy,
    /// The database is not at this code's revision.
    NotMigrated,
    /// Anything else.
    Error,
}

impl Code {
    fn word(self) -> &'static str {
        match self {
            Code::Stale => "stale",
            Code::AlreadySending => "already_sending",
            Code::Invalid => "invalid",
            Code::ReadOnly => "read_only",
            Code::Busy => "busy",
            Code::NotMigrated => "not_migrated",
            Code::Error => "error",
        }
    }
}

/// How a command ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Its exit code.
    pub exit: i32,
    /// Its stdout.
    pub stdout: String,
    /// Its stderr.
    pub stderr: String,
}

impl Reply {
    /// `app --help`.
    pub fn help() -> Reply {
        Reply {
            exit: 0,
            stdout: HELP.to_string(),
            stderr: String::new(),
        }
    }

    /// Bad arguments: empty stdout. `why` names an option, never its value.
    pub fn usage(why: &str) -> Reply {
        Reply {
            exit: 2,
            stdout: String::new(),
            stderr: format!("herdr-inbox app: {why}\n{HELP}"),
        }
    }

    /// Exit 0 with `value` on stdout.
    pub fn json(value: &Value) -> Reply {
        Reply {
            exit: 0,
            stdout: format!("{value}\n"),
            stderr: String::new(),
        }
    }
}

/// Which verb on which row a reply is for.
#[derive(Clone, Debug)]
pub struct Target {
    verb: String,
    entry: String,
}

impl Target {
    /// The target of `request`.
    pub fn of(request: &Request) -> Target {
        Target::new(request.verb.name(), &request.key)
    }

    /// `verb` on row `entry`.
    pub fn new(verb: &str, entry: &str) -> Target {
        Target {
            verb: verb.to_string(),
            entry: entry.to_string(),
        }
    }

    /// Exit 0: it was done.
    pub fn done(&self, done: &Done, delivery: Option<&str>) -> Reply {
        Reply::json(&json!({
            "v": 1,
            "ok": true,
            "verb": self.verb,
            "entry": self.entry,
            "token": done.token,
            "done": done.done,
            "skipped": done.skipped,
            "delivery": delivery,
        }))
    }

    /// Exit 1: nothing was changed.
    pub fn refuse(&self, code: Code, message: &str, closed: Option<&str>) -> Reply {
        let value = json!({
            "v": 1,
            "ok": false,
            "verb": self.verb,
            "entry": self.entry,
            "code": code.word(),
            "message": message,
            "closed": closed,
        });
        Reply {
            exit: 1,
            ..Reply::json(&value)
        }
    }
}

//! A terminal of herdr-core as a delivery sees it (plan herdr-app-inbox §3.2):
//! the agent in it, its input box and the one write, through `hc`.
//!
//! **The address proves nothing.** A terminal's foreground process, read now and
//! checked against the process table, says who runs there, and Claude Code's own
//! record of that exact process says which conversation. Nothing else does, so
//! what cannot be read reads as [`Seen::Unread`]: the answer waits, no try is
//! counted, nothing is typed.
//!
//! **The record is the conversation, followed no further.** herdr's reported
//! session can lag, so a pane follows it through its continuations. The record
//! is rewritten on every session change, so it needs no following, and following
//! it would take a resumed older conversation for the newer one it once continued
//! into and type that one's answer here. Stale, it can only hold an answer back.
//!
//! [`Hosted`] is the real thing, [`NoCore`] a process with no business in a core.

use herdr_agent_kit::agent_procs::claude::{claude_process, nested_claude};
use herdr_agent_kit::agent_procs::{Pid, ProcessTable};
use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;
use inbox_core::deliver::Agent;
use serde_json::Value;

use crate::claude::Transcripts;
use crate::host::core::hc::{Cli, Hc, NO_TERMINAL, Sent, socket};
use crate::host::core::proof::{Refused, foreground};
use crate::host::core::record::Record;
use crate::host::core::{self as core, Machine};

/// Why a Claude that has the terminal cannot be read yet.
pub const NO_RECORD: &str = "Claude Code keeps no record of this process";
/// The record exists and cannot be used.
pub const BAD_RECORD: &str = "Claude Code's record of this process is unusable";
/// A tool of the Claude, or a Claude inside it, has the terminal.
pub const NOT_FOREGROUND: &str = "another program has the terminal";
/// The foreground process is not the one `hc` named.
pub const REPLACED: &str = "the terminal's foreground process just changed";

/// The Claude in a terminal, as of one look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seated {
    /// The agent, as a delivery's decision reads it.
    pub agent: Agent,
    /// Its process.
    pub pid: Pid,
    /// That process's start token.
    pub start: String,
}

/// What one look at a terminal found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// A Claude has the terminal, and its conversation is known.
    Here(Box<Seated>),
    /// The terminal is gone, its program ended, or no Claude is left in it.
    Gone,
    /// Nothing can be said now. Says why.
    Unread(String),
}

/// The terminals of this machine's core.
pub trait Terminals: Send + Sync {
    /// Who is in `terminal` now.
    fn see(&self, terminal: &str) -> Seen;
    /// Its visible screen.
    fn screen(&self, terminal: &str) -> Option<String>;
    /// Types `text`, once `hc` finds it `idle`.
    fn send(&self, terminal: &str, text: &str) -> Sent;
}

/// No core: for a process that delivers only to panes.
pub struct NoCore;

impl Terminals for NoCore {
    fn see(&self, _: &str) -> Seen {
        Seen::Unread("this process has no core".into())
    }
    fn screen(&self, _: &str) -> Option<String> {
        None
    }
    fn send(&self, _: &str, _: &str) -> Sent {
        Sent::Unread("this process has no core")
    }
}

/// The terminals `hc` reports on this machine.
pub struct Hosted<H, M> {
    /// `hc`.
    pub hc: H,
    /// This machine.
    pub machine: M,
}

/// The status word: 0.1.18 prints the 7501 record, 0.1.17 the word alone. `"None"`
/// when the program never said, as a pane's unreported status reads.
fn status(terminal: &Value) -> String {
    let word = match &terminal["status"] {
        Value::Object(record) => record.get("state").and_then(Value::as_str),
        other => other.as_str(),
    };
    word.unwrap_or("None").to_owned()
}

impl<H: Hc, M: Machine + Send + Sync> Hosted<H, M> {
    fn seen(&self, terminal: &str, read: &Value) -> Seen {
        let found = match foreground(read, terminal) {
            Ok(found) => found,
            Err(Refused::NotThatTerminal) => return gone_or_other(read, terminal),
            Err(why) => return Seen::Unread(why.to_string()),
        };
        let table = self.machine.table();
        if !found.matches(&table) {
            return Seen::Unread(REPLACED.into());
        }
        let Some((pid, start)) = claude_process(&table, found.pid) else {
            return Seen::Gone; // a shell again: the Claude left
        };
        if pid != found.pid || nested_claude(&table, pid) {
            return Seen::Unread(NOT_FOREGROUND.into());
        }
        let session = match self.machine.recorded_session(pid, &start) {
            Record::Names(session) => session,
            Record::Missing => return Seen::Unread(NO_RECORD.into()),
            Record::Unusable => return Seen::Unread(BAD_RECORD.into()),
        };
        if self.machine.table().start(pid).as_deref() != Some(&start) {
            return Seen::Unread(REPLACED.into()); // a pid reused during the reads
        }
        Seen::Here(Box::new(Seated {
            agent: Agent {
                kind: Some("claude".into()),
                status: status(read),
                focused: false, // hc reports no viewer; its send waits for a person to pause
                session: Some(session.clone()),
                reported_session: Some(session),
            },
            pid,
            start,
        }))
    }
}

/// `hc` answered for another address, or a finished program.
fn gone_or_other(read: &Value, terminal: &str) -> Seen {
    if read["address"].as_str() == Some(terminal) {
        Seen::Gone
    } else {
        Seen::Unread("hc answered for another terminal".into())
    }
}

impl<H: Hc, M: Machine + Send + Sync> Terminals for Hosted<H, M> {
    fn see(&self, terminal: &str) -> Seen {
        match self.hc.get(terminal) {
            Ok(read) => self.seen(terminal, &read),
            Err(NO_TERMINAL) => Seen::Gone,
            Err(why) => Seen::Unread(why.into()),
        }
    }

    fn screen(&self, terminal: &str) -> Option<String> {
        self.hc.screen(terminal).ok()
    }

    fn send(&self, terminal: &str, text: &str) -> Sent {
        self.hc.send(terminal, text)
    }
}

/// This machine's terminals, as this process's environment places its core.
pub fn live(env: &PluginEnv, herdr: Herdr) -> impl Terminals + use<> {
    Hosted {
        hc: Cli::on_path(socket::from_env(env)),
        machine: core::live::Live {
            config: ProjectsRoot::from_env().and_then(|r| r.path().parent().map(ToOwned::to_owned)),
            files: Transcripts::from_env(),
            state_dir: inbox::state_dir(env),
            herdr,
        },
    }
}

#[cfg(test)]
mod tests;

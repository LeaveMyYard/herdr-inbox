//! Is this process the agent of the terminal it claims? (herdr-panorama
//! `docs/protocol/session-linking.md` §2, read from the caller's end.)
//!
//! The gate there starts from a terminal and looks for its session. Here the
//! caller starts from itself. [`own_claude`]: this process's direct parent is
//! a Claude, and the only one above it. `scripts/launch` `exec`s the binary, so
//! nothing sits between them. That refuses a server under an agent's tools and
//! under any harness the kit does not know, which `claude_process` alone would
//! walk past to the Claude above. Then the terminal's foreground must be that
//! Claude, by pid, name and argv. So a claim copied into another terminal or a
//! pane fails too. The process's own environment is not read: being its child
//! is the stronger proof.
//!
//! A second herdr-core instance with the same `tN` and a colliding pid is
//! the residual Panorama accepts too.

use std::fmt;

use herdr_agent_kit::agent_procs::claude::{claude_process, nested_claude};
use herdr_agent_kit::agent_procs::{Pid, ProcessTable};
use serde_json::Value;

use super::address;
use super::hc::Hc;
use crate::mcp::receipts;

/// Why a claim was not proved. Never shown to the agent: it gets `unavailable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a core address.
    BadAddress,
    /// Another machine's terminal.
    Remote,
    /// No Claude above this process.
    NoClaude,
    /// Something other than the Claude started this process.
    NotItsChild,
    /// A Claude inside a Claude.
    Nested,
    /// `hc` failed, or printed what it should not.
    Hc(&'static str),
    /// `hc` answers for another address, or the terminal is not live.
    NotThatTerminal,
    /// The terminal's foreground is another process.
    NotForeground,
    /// The foreground process has this pid but is not this Claude.
    NotThatProcess,
    /// The process changed during the reads.
    Replaced,
    /// No session to file under.
    NoSession,
    /// Claude Code's record of the process exists and cannot be used.
    BadRecord,
    /// Another live process runs this session.
    Shared,
}

impl Refused {
    /// May the same caller pass in a moment? `hc` slow, gone or still sampling, the foreground
    /// away during a tool, a record mid-rewrite. The rest stay as they are for this process.
    pub fn temporary(self) -> bool {
        matches!(
            self,
            Refused::Hc(_) | Refused::NotForeground | Refused::Replaced | Refused::BadRecord
        )
    }
}

impl Refused {
    /// What the agent reads, from every tool.
    pub fn receipt(self) -> Value {
        match self {
            Refused::Shared => receipts::shared(),
            why if why.temporary() => receipts::undecided(),
            _ => receipts::unproved(),
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refused::BadAddress => "not a core address",
            Refused::Remote => "another machine's terminal",
            Refused::NoClaude => "no Claude above this process",
            Refused::NotItsChild => "this process is not its Claude's own child",
            Refused::Nested => "a nested Claude",
            Refused::Hc(why) => why,
            Refused::NotThatTerminal => "hc reports no such live terminal",
            Refused::NotForeground => "this Claude is not its foreground process",
            Refused::NotThatProcess => "its foreground process is another program",
            Refused::Replaced => "the process changed during the proof",
            Refused::NoSession => "no session",
            Refused::BadRecord => "Claude Code's record of a process is unusable",
            Refused::Shared => "another live process runs this session",
        })
    }
}

/// The process proved to be the terminal's agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proved {
    /// Its pid.
    pub pid: Pid,
    /// Its start token.
    pub start: String,
}

/// The words of an argv: hc's list and the process table's may split
/// differently (`ps` has no argument boundaries), and the kit drops empty ones.
fn words<S: AsRef<str>>(argv: &[S]) -> Vec<String> {
    argv.iter()
        .flat_map(|a| a.as_ref().split_whitespace())
        .map(str::to_owned)
        .collect()
}

/// The foreground process of `hc get`'s `terminal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    /// Its pid.
    pub pid: Pid,
    name: String,
    argv: Vec<String>,
}

impl Foreground {
    /// `table` shows this very program at that pid.
    pub fn matches(&self, table: &impl ProcessTable) -> bool {
        let comm = table.stat(self.pid).map(|s| s.comm);
        comm.as_deref() == Some(&self.name) && words(&table.cmdline(self.pid)) == self.argv
    }
}

/// The foreground process of `hc get`'s `terminal`, or why not.
pub fn foreground(terminal: &Value, claim: &str) -> Result<Foreground, Refused> {
    if terminal["address"].as_str() != Some(claim) || terminal["state"] != "live" {
        return Err(Refused::NotThatTerminal);
    }
    let shape = Refused::Hc("hc get has no foreground process");
    let fg = terminal.get("foreground").ok_or(shape)?;
    let pid = fg["pid"]
        .as_u64()
        .filter(|p| *p > 1)
        .and_then(|p| Pid::try_from(p).ok())
        .ok_or(shape)?;
    let name = fg["name"].as_str().ok_or(shape)?;
    let argv: Vec<&str> = fg["argv"]
        .as_array()
        .ok_or(shape)?
        .iter()
        .map(Value::as_str)
        .collect::<Option<_>>()
        .ok_or(shape)?;
    Ok(Foreground {
        pid,
        name: name.to_owned(),
        argv: words(&argv),
    })
}

/// The Claude this process serves: its direct parent, with no other Claude above.
pub fn own_claude(table: &impl ProcessTable, parent: Pid) -> Result<Proved, Refused> {
    let (pid, start) = claude_process(table, parent).ok_or(Refused::NoClaude)?;
    if pid != parent {
        return Err(Refused::NotItsChild);
    }
    if nested_claude(table, parent) {
        return Err(Refused::Nested);
    }
    Ok(Proved { pid, start })
}

fn plain(claim: &str) -> Result<(), Refused> {
    if !address::valid(claim) {
        return Err(Refused::BadAddress);
    }
    if address::remote(claim) {
        return Err(Refused::Remote);
    }
    Ok(())
}

/// `proved` is the foreground program of terminal `claim`.
fn in_foreground(
    hc: &dyn Hc,
    table: &impl ProcessTable,
    proved: Proved,
    claim: &str,
) -> Result<Proved, Refused> {
    let found = foreground(&hc.get(claim).map_err(Refused::Hc)?, claim)?;
    if found.pid != proved.pid {
        return Err(Refused::NotForeground);
    }
    if !found.matches(table) {
        return Err(Refused::NotThatProcess);
    }
    Ok(proved)
}

/// Proves `claim` for the process whose parent is `parent`.
pub fn prove(
    hc: &dyn Hc,
    table: &impl ProcessTable,
    parent: Pid,
    claim: &str,
) -> Result<Proved, Refused> {
    plain(claim)?;
    let proved = own_claude(table, parent)?;
    in_foreground(hc, table, proved, claim)
}

/// Proves `claim` for a hook: its Claude is the nearest above `parent`, with a
/// shell or two between them (`hooks.json`), and no other Claude above that one.
pub fn prove_hook(
    hc: &dyn Hc,
    table: &impl ProcessTable,
    parent: Pid,
    claim: &str,
) -> Result<Proved, Refused> {
    plain(claim)?;
    let (pid, start) = claude_process(table, parent).ok_or(Refused::NoClaude)?;
    if nested_claude(table, parent) {
        return Err(Refused::Nested);
    }
    in_foreground(hc, table, Proved { pid, start }, claim)
}

#[cfg(test)]
mod tests;

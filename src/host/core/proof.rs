//! Is this process the agent of the terminal it claims? (herdr-panorama
//! `docs/protocol/session-linking.md` §2, read from the caller's end.)
//!
//! The gate there starts from a terminal and looks for its session. Here the
//! caller starts from itself: the nearest Claude above this process must be
//! the terminal's foreground process, by pid, name and argv, and the only
//! Claude above it. So a claim copied into another terminal or a pane, a
//! child of the agent's tools and a nested Claude all fail. The process's
//! own environment is not read: being its descendant is the stronger proof.
//!
//! A second herdr-core instance with the same `tN` and a colliding pid is
//! the residual Panorama accepts too.

use std::fmt;

use herdr_agent_kit::agent_procs::claude::{claude_process, nested_claude};
use herdr_agent_kit::agent_procs::{Pid, ProcessTable};
use serde_json::Value;

use super::address;
use super::hc::Hc;

/// Why a claim was not proved. Never shown to the agent: it gets `unavailable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a core address.
    BadAddress,
    /// Another machine's terminal.
    Remote,
    /// No Claude above this process.
    NoClaude,
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
    /// `hc ls` does not list it once, live.
    NotListed,
    /// The process changed during the reads.
    Replaced,
    /// No session to file under.
    NoSession,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refused::BadAddress => "not a core address",
            Refused::Remote => "another machine's terminal",
            Refused::NoClaude => "no Claude above this process",
            Refused::Nested => "a nested Claude",
            Refused::Hc(why) => why,
            Refused::NotThatTerminal => "hc reports no such live terminal",
            Refused::NotForeground => "this Claude is not its foreground process",
            Refused::NotThatProcess => "its foreground process is another program",
            Refused::NotListed => "hc ls does not list it once, live",
            Refused::Replaced => "the process changed during the proof",
            Refused::NoSession => "no session",
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

/// The foreground process of `hc get`'s `terminal`, or why not.
fn foreground(terminal: &Value, claim: &str) -> Result<(Pid, String, Vec<String>), Refused> {
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
    Ok((pid, name.to_owned(), words(&argv)))
}

/// `hc ls` lists `claim` exactly once, live. Its other fields differ between
/// hc versions (`agent`, and `status` when null), so nothing else is read.
fn listed(list: &Value, claim: &str) -> Result<(), Refused> {
    let terminals = list["terminals"]
        .as_array()
        .ok_or(Refused::Hc("hc ls has no terminals"))?;
    let mut mine = terminals.iter().filter(|t| t["address"] == claim);
    match (mine.next(), mine.next()) {
        (Some(t), None) if t["state"] == "live" => Ok(()),
        _ => Err(Refused::NotListed),
    }
}

/// Proves `claim` for the process whose parent is `parent`.
pub fn prove(
    hc: &dyn Hc,
    table: &impl ProcessTable,
    parent: Pid,
    claim: &str,
) -> Result<Proved, Refused> {
    if !address::valid(claim) {
        return Err(Refused::BadAddress);
    }
    if address::remote(claim) {
        return Err(Refused::Remote);
    }
    let (pid, start) = claude_process(table, parent).ok_or(Refused::NoClaude)?;
    if nested_claude(table, parent) {
        return Err(Refused::Nested);
    }
    let (foreground_pid, name, argv) = foreground(&hc.get(claim).map_err(Refused::Hc)?, claim)?;
    if foreground_pid != pid {
        return Err(Refused::NotForeground);
    }
    let comm = table.stat(pid).map(|s| s.comm);
    if comm.as_deref() != Some(&name) || words(&table.cmdline(pid)) != argv {
        return Err(Refused::NotThatProcess);
    }
    listed(&hc.ls().map_err(Refused::Hc)?, claim)?;
    Ok(Proved { pid, start })
}

#[cfg(test)]
mod tests;

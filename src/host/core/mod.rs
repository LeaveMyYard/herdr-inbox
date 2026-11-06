//! A terminal of herdr-core (herdr-panorama `docs/protocol/core-host.md`).
//!
//! `HC_TERMINAL_ID` is a claim. Any process can set it, and a child inherits
//! it from whatever started it. The caller is the terminal's agent only when
//! [`proof`] says so; the session then comes from Claude Code's own record of
//! that process ([`record`]), else from `CLAUDE_CODE_SESSION_ID`. A caller with
//! no session has nothing to deliver to and no questions to own, so it is
//! refused too.
//!
//! The process table, the record and the transcripts are behind [`Machine`];
//! `hc` is behind [`Hc`]. [`live`] is the real one.

pub mod address;
#[cfg(test)]
pub mod fake;
pub mod hc;
pub mod live;
pub mod proof;
pub mod record;

use std::io::Write;

use herdr_agent_kit::agent_procs::{Pid, ProcessTable};

use super::Host;
use crate::mcp::caller::{Caller, Env};
use crate::mcp::retry::Failure;
use crate::store::hosts::CoreSeat;
use crate::trace;
use hc::Hc;
use proof::{Proved, Refused};

/// This machine, as the proof reads it.
pub trait Machine: Send + Sync {
    /// Its process table.
    type Table: ProcessTable;
    /// The table for one walk: a fresh one each time, so no walk sees an old table.
    fn table(&self) -> Self::Table;
    /// This process's parent, where the walk up starts.
    fn parent(&self) -> Pid;
    /// The session Claude Code's record of process `pid` (start token `start`) names.
    fn recorded_session(&self, pid: Pid, start: &str) -> Option<String>;
    /// The session `reported` runs now (`live_session`).
    fn live(&self, reported: &str) -> String;
}

/// herdr-core.
pub struct Core<H, M> {
    /// `HC_TERMINAL_ID`: the terminal this process says it runs in.
    pub claim: Option<String>,
    /// `hc`.
    pub hc: H,
    /// This machine.
    pub machine: M,
}

impl<H: Hc, M: Machine> Core<H, M> {
    fn proved_caller(&self, claim: &str, env: &Env) -> Result<Caller, Refused> {
        let table = self.machine.table();
        let parent = self.machine.parent();
        let Proved { pid, start } = proof::prove(&self.hc, &table, parent, claim)?;
        let session = self
            .machine
            .recorded_session(pid, &start)
            .or_else(|| env.session_id.clone().filter(|s| !s.is_empty()))
            .ok_or(Refused::NoSession)?;
        // a pid reused during the reads: their answers were about another process
        if self.machine.table().start(pid).as_deref() != Some(&start) {
            return Err(Refused::Replaced);
        }
        let session = Some(self.machine.live(&session)).filter(|s| !s.is_empty());
        Ok(Caller {
            session_id: session,
            cwd: env.cwd.clone(),
            core: Some(CoreSeat {
                terminal: claim.to_owned(),
                machine: None,
            }),
            ..Caller::default()
        })
    }
}

impl<H: Hc, M: Machine> Host for Core<H, M> {
    fn caller(&self, env: &Env) -> Result<Option<Caller>, Failure> {
        let Some(claim) = self.claim.as_deref().filter(|c| !c.is_empty()) else {
            return Ok(None);
        };
        match trace::timed("caller.core_proof", || self.proved_caller(claim, env)) {
            Ok(caller) => Ok(Some(caller)),
            Err(why) => {
                let _ = writeln!(std::io::stderr(), "herdr-inbox: core terminal: {why}");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests;

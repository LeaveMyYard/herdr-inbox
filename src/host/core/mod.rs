//! A terminal of herdr-core (herdr-panorama `docs/protocol/core-host.md`).
//!
//! `HC_TERMINAL_ID` is a claim. Any process can set it, and a child inherits
//! it from whatever started it. The caller is the terminal's agent only when
//! [`proof`] says so; the session then comes from Claude Code's own record of
//! that process ([`record`]), else from `CLAUDE_CODE_SESSION_ID`.
//!
//! **A refused claim is never a guess at another identity.** A core gives its
//! own environment to every terminal, so a process here can carry a pane's
//! `HERDR_PANE_ID` and socket, and the pane path would take that pane's agent
//! and session on those variables alone. So with a claim present the pane
//! path runs only if herdr itself lists this process's Claude in that pane's
//! foreground ([`pane`]). Otherwise every tool answers `unavailable`, whether the
//! reason is a forgery or a slow `hc`: [`Failure::Undecided`] when it may pass
//! in a moment, which says to call again, else [`Failure::Unproved`].
//!
//! The process table, the record and the transcripts are behind [`Machine`];
//! `hc` is behind [`Hc`]. [`live`] is the real one.

pub mod address;
#[cfg(test)]
pub mod fake;
pub mod hc;
pub mod live;
pub mod pane;
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
use record::Record;

/// This machine, as the proof reads it.
pub trait Machine: Send + Sync {
    /// Its process table.
    type Table: ProcessTable;
    /// The table for one walk: a fresh one each time, so no walk sees an old table.
    fn table(&self) -> Self::Table;
    /// This process's parent, where the walk up starts.
    fn parent(&self) -> Pid;
    /// Claude Code's record of process `pid` (start token `start`).
    fn recorded_session(&self, pid: Pid, start: &str) -> Record;
    /// The session `reported` runs now (`live_session`).
    fn live(&self, reported: &str) -> String;
    /// herdr's foreground processes of `pane`; `None` when herdr gives no list.
    fn pane_foreground(&self, pane: &str) -> Option<Vec<Pid>>;
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
        let session = match self.machine.recorded_session(pid, &start) {
            Record::Names(session) => session,
            Record::Unusable => return Err(Refused::BadRecord),
            Record::Missing => env
                .session_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or(Refused::NoSession)?,
        };
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
                if pane::proved(&self.machine, env) {
                    Ok(None) // really this pane's agent: the pane path answers
                } else if why.temporary() {
                    Err(Failure::Undecided)
                } else {
                    Err(Failure::Unproved)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

//! Does another live process run this session? (`claude --resume <id>` of a
//! session still running elsewhere starts with no warning.)
//!
//! Ownership is the session, so two proved agents with one id are one owner:
//! one would take the other's answers and withdraw its questions. Which copy
//! is asking cannot be told, and the two have diverged, so neither is served
//! until one ends. There is no tie-break: both starting together refuse alike.
//!
//! A record proves its process live by `procStart`, so one left by a dead
//! process, or by a pid since reused, refuses nothing. A file that cannot be
//! read refuses only while a live Claude owns its name: then it is that process's
//! record mid-rewrite, and its session is unknown for a moment. Records of
//! another kind are no agent a terminal holds.

use herdr_agent_kit::agent_procs::claude::is_claude;
use herdr_agent_kit::agent_procs::{Pid, ProcessTable};

use super::proof::Refused;
use super::record::Other;

/// Refuses if any of `others` is a live process on one of `sessions`.
pub fn check(
    table: &impl ProcessTable,
    others: &[Other],
    sessions: &[&str],
) -> Result<(), Refused> {
    for other in others {
        match other {
            Other::Runs(pid, start, session)
                if sessions.contains(&session.as_str())
                    && table.start(*pid).as_deref() == Some(start) =>
            {
                return Err(Refused::Shared);
            }
            Other::Unreadable(pid) if a_live_claude(table, *pid) => {
                return Err(Refused::BadRecord);
            }
            _ => {}
        }
    }
    Ok(())
}

fn a_live_claude(table: &impl ProcessTable, pid: Pid) -> bool {
    table
        .stat(pid)
        .is_some_and(|st| is_claude(&st.comm, &table.cmdline(pid)))
}

#[cfg(test)]
mod tests;

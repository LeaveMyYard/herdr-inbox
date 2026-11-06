//! Does a process that carries a core claim really run in the pane it names?
//!
//! `HERDR_PANE_ID` is inherited like any variable, so it proves nothing here.
//! herdr's own list does: the pane's foreground holds this process's Claude,
//! which a core terminal's child cannot pass, since its Claude is in no pane.
//! This is what keeps a herdr TUI started inside a core terminal working.

use herdr_agent_kit::agent_procs::ProcessTable;

use super::Machine;
use super::proof::own_claude;
use crate::mcp::caller::Env;

/// `env`'s pane is proved to hold this process's Claude.
pub fn proved(machine: &impl Machine, env: &Env) -> bool {
    let Some(pane) = env.pane_id.as_deref().filter(|p| !p.is_empty()) else {
        return false;
    };
    let Ok(claude) = own_claude(&machine.table(), machine.parent()) else {
        return false;
    };
    machine
        .pane_foreground(pane)
        .is_some_and(|pids| pids.contains(&claude.pid))
        && machine.table().start(claude.pid).as_deref() == Some(&claude.start)
}

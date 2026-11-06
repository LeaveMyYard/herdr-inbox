//! Which agent is this hook's? The hooks' first gate. `HC_TERMINAL_ID` is a claim
//! until proved, like any other (`proof`), and a hook's Claude is its nearest
//! above a shell or two, so the proof is `prove_hook`'s. What cannot be proved now
//! gets nothing from this firing, and the worker's typing, which proves its own
//! terminal, is still there.
//!
//! **A hook that carries answers obeys [`twins`].** Its session is the owner, and
//! if another live process runs that session, one of them would be woken with the
//! other's answer and mark it seen. The rules a session starts with are nobody's
//! answer, so those go through.
//!
//! **A refused claim is never a pane by default.** A core gives its environment to
//! its terminals, so `HERDR_PANE_ID` proves nothing beside a claim: herdr must list
//! this Claude in that pane's foreground, as `host::core::pane` asks of a server.

use std::path::PathBuf;

use herdr_agent_kit::agent_procs::claude::claude_process;
use herdr_agent_kit::agent_procs::{ProcessTable, SystemProcesses, parent_pid};
use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::PaneId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::plugin_env::PluginEnv;

use super::hc::{Cli, socket};
use super::proof::{Proved, prove_hook};
use super::{record, twins};

/// Whose hook this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seat {
    /// The proved agent of this core terminal.
    Terminal(String),
    /// A herdr pane's agent.
    Pane,
    /// No host's own agent: nothing for it.
    Nobody,
}

fn config() -> Option<PathBuf> {
    let root = ProjectsRoot::from_env()?;
    root.path().parent().map(ToOwned::to_owned)
}

/// Another live process runs `session`.
fn twinned(table: &impl ProcessTable, proved: &Proved, session: &str) -> bool {
    let others = config().map_or_else(Vec::new, |c| record::others(&c, proved.pid));
    twins::check(table, &others, &[session]).is_err()
}

/// herdr lists this hook's Claude in the foreground of the pane `env` names.
fn in_its_pane(env: &PluginEnv, table: &impl ProcessTable) -> bool {
    let Some(pane) = env.pane_id().filter(|p| !p.is_empty()) else {
        return false;
    };
    let Some((claude, _)) = claude_process(table, parent_pid()) else {
        return false;
    };
    let info = Herdr::from_env().process_info(&PaneId::new(pane)).ok();
    let foreground = info
        .and_then(|i| i.foreground_processes)
        .unwrap_or_default();
    foreground
        .iter()
        .filter_map(|p| p.pid)
        .any(|p| p == i64::from(claude))
}

/// The seat of a hook of `session`. `carries`: it may hand an answer over.
pub fn seat(env: &PluginEnv, session: Option<&str>, carries: bool) -> Seat {
    let Some(claim) = env.var("HC_TERMINAL_ID").filter(|c| !c.is_empty()) else {
        return if env.pane_id().is_some() {
            Seat::Pane
        } else {
            Seat::Nobody
        };
    };
    let hc = Cli::on_path(socket::from_env(env));
    let table = SystemProcesses::new();
    match prove_hook(&hc, &table, parent_pid(), claim) {
        Ok(proved) if carries && session.is_some_and(|s| twinned(&table, &proved, s)) => {
            Seat::Nobody
        }
        Ok(_) => Seat::Terminal(claim.to_owned()),
        Err(_) if in_its_pane(env, &table) => Seat::Pane,
        Err(_) => Seat::Nobody,
    }
}

//! Is this hook the agent's of a core terminal? The hooks' first gate, as
//! `HERDR_PANE_ID` is for a pane, and a claim until proved like any other
//! (`proof`). A hook's Claude is its nearest above a shell or two, so the proof
//! is `prove_hook`'s. What cannot be proved now gets nothing from this firing,
//! and the worker's typing, which proves its own terminal, is still there.

use herdr_agent_kit::agent_procs::{SystemProcesses, parent_pid};
use herdr_agent_kit::plugin_env::PluginEnv;

use super::hc::{Cli, socket};
use super::proof::prove_hook;

/// `HC_TERMINAL_ID`, once proved to be this hook's Claude's terminal.
pub fn proved_terminal(env: &PluginEnv) -> Option<String> {
    let claim = env.var("HC_TERMINAL_ID").filter(|c| !c.is_empty())?;
    let hc = Cli::on_path(socket::from_env(env));
    let proved = prove_hook(&hc, &SystemProcesses::new(), parent_pid(), claim);
    proved.ok().map(|_| claim.to_owned())
}

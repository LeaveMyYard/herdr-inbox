//! Where an agent runs (plan herdr-app-inbox §3.2, B-host): the seam between
//! "who is asking" and the hosts that can say. [`tui`] is a herdr pane, as it
//! always was; [`core`](mod@core) is a terminal of herdr-core.
//!
//! A host speaks only for a caller it can stand behind. A core claim that
//! fails its proof falls through to the pane path, so a process with a pane
//! stays exactly what it was, and one with neither gets `unavailable`.

pub mod core;
pub mod tui;

use crate::mcp::caller::{Caller, Env};
use crate::mcp::retry::Failure;

/// One place an agent can run.
pub trait Host: Send + Sync {
    /// The caller if it is this host's agent, proved; `None` if not.
    fn caller(&self, env: &Env) -> Result<Option<Caller>, Failure>;
}

/// Who is asking: the first of `hosts` that holds the caller.
pub fn caller(hosts: &[Box<dyn Host>], env: &Env) -> Result<Caller, Failure> {
    for host in hosts {
        if let Some(caller) = host.caller(env)? {
            return Ok(caller);
        }
    }
    Ok(Caller::default())
}

#[cfg(test)]
mod tests;

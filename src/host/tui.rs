//! A herdr pane: [`crate::mcp::caller::resolve`], unchanged. It always answers,
//! a caller with no pane included, so it comes last.

use super::Host;
use crate::mcp::caller::{Caller, Env, World, resolve};
use crate::mcp::retry::Failure;

/// The herdr TUI.
pub struct Tui {
    /// herdr and the process tree.
    pub world: Box<dyn World + Send + Sync>,
}

impl Host for Tui {
    fn caller(&self, env: &Env) -> Result<Option<Caller>, Failure> {
        resolve(&*self.world, env).map(Some)
    }
}

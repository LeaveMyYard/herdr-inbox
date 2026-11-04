//! What the worker asks herdr about its own life (Python `herdr.server_gone`,
//! `_listed_root`).

use std::path::PathBuf;

use herdr_agent_kit::herdr_api::HerdrError;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::internal::plugin_paths::INBOX_ID;

/// herdr says this server is not running ([`HerdrError::NotRunning`]: its
/// socket is missing or refuses connections, Python's `server_not_running`).
/// Any other failure (a timeout, another error, a garbled reply) is no proof
/// that the server is gone.
pub fn server_gone(herdr: &Herdr) -> bool {
    matches!(herdr.workspaces(), Err(HerdrError::NotRunning))
}

/// The inbox's `plugin_root` as this server lists it now (`None`: not
/// listed, listed without a root, or no answer).
pub fn listed_root(herdr: &Herdr) -> Option<PathBuf> {
    herdr
        .plugins()
        .ok()?
        .into_iter()
        .filter(|p| p.plugin_id.as_deref() == Some(INBOX_ID))
        .find_map(|p| p.plugin_root.filter(|root| !root.is_empty()))
        .map(PathBuf::from)
}

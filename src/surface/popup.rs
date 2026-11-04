//! Where the popup opens from (Python `herdr.open_popup`'s inputs): the
//! inbox's own root, its command's working directory, and the pane that was
//! focused when herdr ran the command that opens it.

use std::path::{Path, PathBuf};

use herdr_agent_kit::plugin_env::PluginEnv;
use serde_json::Value;

/// The inbox's herdr plugin id (Python `herdr.INBOX_PLUGIN_ID`).
pub const INBOX_PLUGIN_ID: &str = "leavemyyard.herdr-inbox";
/// The popup's entrypoint.
pub const ENTRYPOINT: &str = "inbox";
/// The variable naming the operator's pane for the popup.
pub const FOCUS_ENV: &str = "INBOX_FOCUS_PANE";

/// The checkout holding `exe` (Python `herdr._own_checkout`): its nearest
/// ancestor with a `herdr-plugin.toml`, e.g. a linked checkout's
/// `target/release/herdr-inbox`. Never the caller's: `inbox open` may run
/// from another plugin's action, whose `HERDR_PLUGIN_ROOT` is its own.
pub fn own_checkout(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .find(|dir| dir.join("herdr-plugin.toml").is_file())
        .map(Path::to_path_buf)
}

/// The pane focused when herdr ran this plugin command (Python
/// `herdr.invocation_pane`, then `HERDR_PANE_ID`):
/// `HERDR_PLUGIN_CONTEXT_JSON`'s `focused_pane_id`, the documented source
/// for actions and pane commands alike.
pub fn focused_pane(env: &PluginEnv) -> Option<String> {
    let context = env
        .plugin_context_json()
        .and_then(|j| serde_json::from_str::<Value>(j).ok());
    context
        .as_ref()
        .and_then(|c| c.get("focused_pane_id"))
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| env.pane_id().filter(|p| !p.is_empty()).map(str::to_string))
}

#[cfg(test)]
mod tests;

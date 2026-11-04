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

/// The inbox checkout holding `exe` (Python `herdr._own_checkout`): its
/// nearest ancestor (symlinks resolved) whose `herdr-plugin.toml` is the
/// inbox's own (`id = "leavemyyard.herdr-inbox"`), e.g. a linked checkout's
/// `target/release/herdr-inbox`. Never another plugin's: `inbox open` may run
/// from another plugin's action, and a binary may sit under another
/// plugin's tree (a shared target dir).
pub fn own_checkout(exe: &Path) -> Option<PathBuf> {
    let exe = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    exe.ancestors()
        .skip(1)
        .find(|dir| is_inbox_manifest(&dir.join("herdr-plugin.toml")))
        .map(Path::to_path_buf)
}

/// The manifest's top-level `id` is the inbox's.
fn is_inbox_manifest(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let id = text
        .lines()
        .take_while(|l| !l.trim_start().starts_with('['))
        .find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "id").then(|| value.trim().trim_matches('"').to_string())
        });
    id.as_deref() == Some(INBOX_PLUGIN_ID)
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

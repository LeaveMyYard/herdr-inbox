//! The popup's herdr calls over the kit's client (Python `herdr.py`):
//! where the operator is (`invocation_pane`, `focused_pane`) and going to
//! an agent (`live_agent`'s status, `focus_agent`).

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{AgentStatus, PaneId};
use herdr_agent_kit::plugin_env::PluginEnv;
use serde_json::Value;

use crate::tui::goto::Agents;

/// The pane focused when herdr ran the popup (Python
/// `herdr.invocation_pane`): `HERDR_PLUGIN_CONTEXT_JSON`'s
/// `focused_pane_id`. Not `HERDR_PANE_ID`: in a popup that is the popup.
pub fn invocation_pane(env: &PluginEnv) -> Option<String> {
    let context = env
        .plugin_context_json()
        .and_then(|j| serde_json::from_str::<Value>(j).ok())?;
    context
        .get("focused_pane_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The pane herdr reports focused (Python `herdr.focused_pane`).
pub fn focused_pane(herdr: &Herdr) -> Option<String> {
    let panes = herdr.panes().ok()?;
    panes
        .into_iter()
        .find(|p| p.focused)
        .and_then(|p| p.pane_id)
        .map(|p| p.to_string())
}

/// Going to agents over herdr.
pub struct LiveAgents(pub Herdr);

impl Agents for LiveAgents {
    fn blocked(&self, pane: &str) -> bool {
        matches!(
            self.0.agent(&PaneId::new(pane)),
            Ok(Some(a)) if a.agent_status == AgentStatus::Blocked
        )
    }

    fn focus(&self, pane: &str) -> Option<String> {
        self.0
            .focus_agent(&PaneId::new(pane))
            .err()
            .map(|e| e.to_string())
    }
}

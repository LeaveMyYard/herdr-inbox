//! One agent as `herdr agent list` reports it, for the blocked scan.

use serde_json::Value as Json;

/// One agent of `herdr agent list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenAgent {
    /// Its pane.
    pub pane_id: String,
    /// `agent_status` (`blocked`, `working`, ...).
    pub status: Option<String>,
    /// Its tab, as herdr gave it (stored so; an empty one is not looked up).
    pub tab_id: Option<String>,
    /// `agent_session.value`.
    pub session: Option<String>,
    /// `agent` (its kind).
    pub agent: Option<String>,
}

impl SeenAgent {
    /// One entry of the list; `None` for one Python skips (not an object, no pane).
    pub fn from_json(v: &Json) -> Option<SeenAgent> {
        let text = |k: &str| v.get(k).and_then(Json::as_str).map(String::from);
        let pane_id = text("pane_id").filter(|p| !p.is_empty())?;
        Some(SeenAgent {
            pane_id,
            status: text("agent_status"),
            tab_id: text("tab_id"),
            session: v
                .get("agent_session")
                .and_then(|s| s.get("value"))
                .and_then(Json::as_str)
                .map(String::from),
            agent: text("agent"),
        })
    }
}

//! A conversation that moved to another pane (Python `service.session_pane`,
//! `_relocate`, `_tab_name`): the one pane whose agent runs the asker's
//! conversation now, and the move there.

use herdr_agent_kit::herdr_api::{AgentInfo, TabId};

use super::Ctx;
use crate::claude::{chain, live_session};
use crate::store::sessions::session_transcript;
use crate::tabs::{DEFAULT_PREFIX, base_label};

/// "moved: the asking agent is now in pane" (Python `MOVED`).
pub const MOVED: &str = "moved: the asking agent is now in pane";

/// The tab's label without the inbox's own mark (Python `_tab_name`).
pub fn tab_name(ctx: &Ctx<'_>, tab: Option<&str>) -> Option<String> {
    let tab = tab.filter(|t| !t.is_empty())?;
    let label = ctx
        .herdr
        .tab(&TabId::new(tab))
        .ok()?
        .label
        .filter(|l| !l.is_empty())?;
    Some(base_label(&label, &[DEFAULT_PREFIX]))
}

/// The one pane (and its tab) whose agent runs `session`'s conversation now,
/// continuations followed on both sides; `None` when none or several do.
pub fn session_pane(
    ctx: &Ctx<'_>,
    session: Option<&str>,
    agents: &[AgentInfo],
) -> rusqlite::Result<Option<(String, Option<String>)>> {
    let Some(session) = session.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let db = ctx.store.db();
    let known = session_transcript(db, session)?;
    let live = chain(ctx.files, session, known.as_deref())
        .pop()
        .unwrap_or_else(|| session.to_string());
    let mut hits = Vec::new();
    for a in agents {
        let reported = a
            .agent_session
            .as_ref()
            .and_then(|s| s.value.as_deref())
            .filter(|v| !v.is_empty());
        let pane = a
            .pane_id
            .as_ref()
            .map(|p| p.as_str().to_string())
            .filter(|p| !p.is_empty());
        if let (Some(reported), Some(pane)) = (reported, pane)
            && live_session(ctx.files, ctx.state_dir, reported, Some(&pane)) == live
        {
            hits.push((pane, a.tab_id.as_ref().map(|t| t.as_str().to_string())));
        }
    }
    Ok(if hits.len() == 1 { hits.pop() } else { None })
}

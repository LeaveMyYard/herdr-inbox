//! Who is asking (Python `service.resolve_caller`), resolved per call:
//! `/clear` changes the session and panes move.
//!
//! - The pane is `HERDR_PANE_ID`, unless this server runs under a Claude that
//!   was itself started by the pane's agent (a nested Claude is not the pane's
//!   agent): then there is no pane.
//! - herdr names the pane's tab and the session its agent reported. A pane
//!   herdr no longer knows (herdr gives a moved pane a new id, live 2026-10-17)
//!   is replaced by the one pane whose agent runs this conversation now.
//! - The session is herdr's, else `CLAUDE_CODE_SESSION_ID`, followed to the
//!   session the conversation lives in now (`live_session`, D6).
//! - The tab label is herdr's without the inbox's own `❓N ` marks.
//!
//! herdr and the process tree are behind [`World`]; [`live`] is the real one.

pub mod live;

use crate::mcp::retry::Failure;
use crate::store::questions::Owner;
use crate::trace;

/// The asker (Python `service.Caller`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller {
    pub pane_id: Option<String>,
    pub tab_id: Option<String>,
    pub tab_label: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
}

impl Caller {
    /// Whose questions and alerts are the caller's (its session's, else its pane's).
    pub fn owner(&self) -> Owner<'_> {
        Owner {
            session_id: self.session_id.as_deref(),
            pane_id: self.pane_id.as_deref(),
        }
    }
}

/// What this process knows of itself (its environment and directory).
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// `HERDR_PANE_ID`: the pane this process STARTED in.
    pub pane_id: Option<String>,
    /// `CLAUDE_CODE_SESSION_ID`.
    pub session_id: Option<String>,
    /// `HERDR_TAB_ID`.
    pub tab_id: Option<String>,
    /// The working directory.
    pub cwd: Option<String>,
}

/// A pane as herdr describes it (Python `herdr.pane_identity`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pane {
    pub tab_id: Option<String>,
    /// The session its agent reported (an empty one is none).
    pub session: Option<String>,
}

/// herdr, the process tree, the transcripts and the tab marks, as the
/// resolution reads them. Every lookup is failure-tolerant as in Python
/// (herdr unreachable reads as "no such pane"), except [`World::marks`],
/// which opens the store and fails as a tool's store does.
pub trait World {
    /// A Claude runs this process inside another Claude (Python `proc.nested_claude`).
    fn nested(&self) -> bool;
    /// `pane get`: `None` when herdr has no such pane, or cannot be asked.
    fn pane(&self, pane: &str) -> Option<Pane>;
    /// `tab get`'s label.
    fn tab_label(&self, tab: &str) -> Option<String>;
    /// `agent list`: each agent's pane and reported session (empty when herdr cannot be asked).
    fn agents(&self) -> Vec<(String, Option<String>)>;
    /// The session `reported` runs now (Python `store.live_session`), as seen from `pane`.
    fn live(&self, reported: &str, pane: Option<&str>) -> String;
    /// The marks of ours that may be stripped from `tab`'s label (Python `tabs.marks_for`).
    fn marks(&self, tab: Option<&str>) -> Result<Vec<String>, Failure>;
}

fn some(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The one pane whose agent runs `session`'s conversation (continuations
/// followed on both sides), or `None` (Python `service._pane_running`).
pub fn pane_running(world: &dyn World, session: Option<&str>) -> Option<String> {
    let session = session.filter(|s| !s.is_empty())?;
    let live = world.live(session, None);
    let hits: Vec<String> = world
        .agents()
        .into_iter()
        .filter(|(pane, reported)| {
            !pane.is_empty()
                && reported
                    .as_deref()
                    .is_some_and(|r| !r.is_empty() && world.live(r, Some(pane)) == live)
        })
        .map(|(pane, _)| pane)
        .collect();
    match hits.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// Who is asking now.
pub fn resolve(world: &dyn World, env: &Env) -> Result<Caller, Failure> {
    let mut pane_id = some(env.pane_id.as_deref());
    if trace::timed("caller.nested_claude", || {
        pane_id.is_some() && world.nested()
    }) {
        pane_id = None; // a claude run by the pane's agent is not the pane's agent
    }
    let mut ident = trace::timed("caller.pane_identity", || {
        pane_id.as_deref().and_then(|p| world.pane(p))
    });
    if pane_id.is_some()
        && ident.is_none()
        && let Some(moved) = pane_running(world, env.session_id.as_deref())
    {
        ident = world.pane(&moved);
        pane_id = Some(moved);
    }
    let ident = ident.unwrap_or_default();
    let tab_label = ident
        .tab_id
        .as_deref()
        .filter(|t| !t.is_empty()) // Python `if out["tab_id"]`
        .and_then(|t| world.tab_label(t));
    let session_id = some(ident.session.as_deref())
        .or_else(|| some(env.session_id.as_deref()))
        .map(|s| world.live(&s, pane_id.as_deref()))
        .filter(|s| !s.is_empty());
    let tab_label = match tab_label {
        // never our "❓2 " prefix
        Some(label) => {
            let marks = trace::timed("caller.base_label_store", || {
                world.marks(ident.tab_id.as_deref())
            })?;
            let marks: Vec<&str> = marks.iter().map(String::as_str).collect();
            Some(crate::tabs::base_label(&label, &marks))
        }
        None => None,
    };
    Ok(Caller {
        tab_id: some(ident.tab_id.as_deref()).or_else(|| some(env.tab_id.as_deref())),
        pane_id,
        tab_label,
        session_id,
        cwd: env.cwd.clone(),
    })
}

#[cfg(test)]
mod tests;

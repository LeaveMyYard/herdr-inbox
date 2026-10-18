//! The pane's agent as the watcher sees it (Python `herdr.live_agent`): the
//! live session (continuations followed, `claude::live_session`) and status.

use std::path::Path;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{AgentStatus, PaneId};
use inbox_core::rewake::Status;

use crate::claude::{Transcripts, live_session};

/// The agent in `pane`: its live session (`None` when herdr names none) and
/// status; `None` when there is no agent, or herdr cannot be asked (the
/// watch then ends, as in Python).
pub fn pane_agent(
    herdr: &Herdr,
    files: &Transcripts,
    state_dir: &Path,
    pane: &str,
) -> Option<(Option<String>, Status)> {
    let agent = herdr.agent(&PaneId::new(pane)).ok()??;
    let reported = agent
        .agent_session
        .and_then(|s| s.value)
        .filter(|v| !v.is_empty());
    let live = reported.map(|r| live_session(files, state_dir, &r, Some(pane)));
    Some((live, status(agent.agent_status)))
}

/// herdr's status, as the policy reads it.
pub fn status(s: AgentStatus) -> Status {
    match s {
        AgentStatus::Idle => Status::Idle,
        AgentStatus::Done => Status::Done,
        AgentStatus::Working => Status::Working,
        AgentStatus::Blocked => Status::Blocked,
        _ => Status::Other,
    }
}

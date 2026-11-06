//! The agent as the watcher sees it: a pane's (Python `herdr.live_agent`) with
//! the live session (continuations followed, `claude::live_session`) and status,
//! or a core terminal's ([`crate::delivery::terminal`]).

use std::path::Path;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{AgentStatus, PaneId};
use inbox_core::rewake::Status;

use crate::claude::{Transcripts, live_session};
use crate::delivery::terminal::{Seen, Terminals};

/// The agent in `pane`: its live session (`None` when herdr names none) and
/// status; `None` when there is no agent, it answers for another pane
/// (herdr's target also matches agent names, #97), or herdr cannot be asked
/// (the watch then ends, as in Python).
pub fn pane_agent(
    herdr: &Herdr,
    files: &Transcripts,
    state_dir: &Path,
    pane: &str,
) -> Option<(Option<String>, Status)> {
    let agent = herdr.agent(&PaneId::new(pane)).ok()??;
    if agent.pane_id.as_ref().is_some_and(|p| p.as_str() != pane) {
        return None;
    }
    let reported = agent
        .agent_session
        .and_then(|s| s.value)
        .filter(|v| !v.is_empty());
    let live = reported.map(|r| live_session(files, state_dir, &r, Some(pane)));
    Some((live, status(agent.agent_status)))
}

/// The agent in core `terminal`, as [`pane_agent`] says it. One that cannot be
/// read now names no session: the watch waits, and ends only after a long while.
pub fn terminal_agent(core: &dyn Terminals, terminal: &str) -> Option<(Option<String>, Status)> {
    match core.see(terminal) {
        Seen::Here(seated) => Some((seated.agent.session, word(&seated.agent.status))),
        Seen::Gone => None,
        Seen::Unread(_) => Some((None, Status::Other)),
    }
}

fn word(status: &str) -> Status {
    match status {
        "idle" => Status::Idle,
        "done" => Status::Done,
        "working" => Status::Working,
        "blocked" => Status::Blocked,
        _ => Status::Other,
    }
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

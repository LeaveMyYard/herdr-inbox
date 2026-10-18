//! Where a session's transcript is (Python `transcript.find`).

use std::path::{Path, PathBuf};

use herdr_agent_kit::agent_session::SessionId;
use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;

/// Transcript lookups under one projects root
/// (`${CLAUDE_CONFIG_DIR:-~/.claude}/projects`).
#[derive(Clone, Debug)]
pub struct Transcripts {
    root: Option<ProjectsRoot>,
}

impl Transcripts {
    /// Lookups under `root`; with `None` (no home directory) only recorded
    /// paths are found.
    pub fn new(root: Option<ProjectsRoot>) -> Transcripts {
        Transcripts { root }
    }

    /// Lookups under this process's Claude Code projects directory.
    pub fn from_env() -> Transcripts {
        Transcripts::new(ProjectsRoot::from_env())
    }

    /// The transcript of `session_id`: `known` (the path the SessionStart
    /// hook recorded) while it is a file, else `<root>/*/<session>.jsonl`.
    /// Only a canonical session id is looked up by name, so a stored value
    /// never becomes a pattern or a path.
    pub fn find(&self, session_id: &str, known: Option<&str>) -> Option<PathBuf> {
        if let Some(known) = known.filter(|k| !k.is_empty() && Path::new(k).is_file()) {
            return Some(PathBuf::from(known));
        }
        let id = SessionId::parse(session_id)?;
        self.root.as_ref()?.find(&id, None)
    }
}

#[cfg(test)]
mod tests;

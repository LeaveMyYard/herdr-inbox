//! The real [`Machine`]: this process's ancestors, Claude Code's records and
//! the transcripts.

use std::path::PathBuf;

use herdr_agent_kit::agent_procs::{Pid, SystemProcesses, parent_pid};

use super::{Machine, record};
use crate::claude::{Transcripts, live_session};

/// This machine as this process finds it.
pub struct Live {
    /// Claude Code's config directory.
    pub config: Option<PathBuf>,
    /// The transcripts.
    pub files: Transcripts,
    /// The inbox's state directory (`starts/` resume markers).
    pub state_dir: PathBuf,
}

impl Machine for Live {
    type Table = SystemProcesses;

    fn table(&self) -> SystemProcesses {
        SystemProcesses::new()
    }

    fn parent(&self) -> Pid {
        parent_pid()
    }

    fn recorded_session(&self, pid: Pid, start: &str) -> Option<String> {
        record::session(self.config.as_deref()?, pid, start)
    }

    fn live(&self, reported: &str) -> String {
        live_session(&self.files, &self.state_dir, reported, None)
    }
}

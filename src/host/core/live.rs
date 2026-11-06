//! The real [`Machine`]: this process's ancestors, herdr's panes, Claude
//! Code's records and the transcripts.

use std::path::PathBuf;

use herdr_agent_kit::agent_procs::{Pid, SystemProcesses, parent_pid};
use herdr_agent_kit::herdr_api::PaneId;
use herdr_agent_kit::herdr_api::blocking::Herdr;

use super::Machine;
use super::record::{self, Other, Record};
use crate::claude::{Transcripts, live_session};

/// This machine as this process finds it.
pub struct Live {
    /// Claude Code's config directory.
    pub config: Option<PathBuf>,
    /// The transcripts.
    pub files: Transcripts,
    /// The inbox's state directory (`starts/` resume markers).
    pub state_dir: PathBuf,
    /// herdr, for the pane proof.
    pub herdr: Herdr,
}

impl Machine for Live {
    type Table = SystemProcesses;

    fn table(&self) -> SystemProcesses {
        SystemProcesses::new()
    }

    fn parent(&self) -> Pid {
        parent_pid()
    }

    fn recorded_session(&self, pid: Pid, start: &str) -> Record {
        match self.config.as_deref() {
            Some(config) => record::session(config, pid, start),
            None => Record::Missing,
        }
    }

    fn other_records(&self, pid: Pid) -> Vec<Other> {
        match self.config.as_deref() {
            Some(config) => record::others(config, pid),
            None => Vec::new(),
        }
    }

    fn pane_foreground(&self, pane: &str) -> Option<Vec<Pid>> {
        let info = self.herdr.process_info(&PaneId::new(pane)).ok()?;
        let foreground = info.foreground_processes?;
        let pids = foreground.iter().filter_map(|p| p.pid);
        Some(pids.filter_map(|p| Pid::try_from(p).ok()).collect())
    }

    fn live(&self, reported: &str) -> String {
        live_session(&self.files, &self.state_dir, reported, None)
    }
}

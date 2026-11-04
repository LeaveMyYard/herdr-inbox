//! The real [`World`]: herdr over its socket (kit `blocking::Herdr`), this
//! process's ancestors, the Claude transcripts and the inbox's own store.

use std::path::PathBuf;

use herdr_agent_kit::agent_procs::claude::nested_claude_here;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{PaneId, TabId};

use super::{Pane, World};
use crate::claude::{Transcripts, live_session};
use crate::mcp::open::{failure, open};
use crate::mcp::retry::Failure;
use crate::tabs::label::marks_for;

/// herdr, transcripts and the store as this process finds them.
pub struct Live {
    pub herdr: Herdr,
    pub files: Transcripts,
    /// The inbox's state directory (`starts/` resume markers).
    pub state_dir: PathBuf,
    /// The inbox database.
    pub db: PathBuf,
}

impl World for Live {
    fn nested(&self) -> bool {
        nested_claude_here()
    }

    fn pane(&self, pane: &str) -> Option<Pane> {
        let info = self.herdr.pane(&PaneId::new(pane)).ok()?;
        Some(Pane {
            tab_id: info.tab_id.map(|t| t.as_str().to_owned()),
            session: info
                .agent_session
                .as_ref()
                .and_then(|s| s.value())
                .map(str::to_owned),
        })
    }

    fn tab_label(&self, tab: &str) -> Option<String> {
        self.herdr.tab(&TabId::new(tab)).ok()?.label
    }

    fn agents(&self) -> Vec<(String, Option<String>)> {
        self.herdr
            .agents()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| {
                let session = a.session_value().map(str::to_owned);
                Some((a.pane_id?.as_str().to_owned(), session))
            })
            .collect()
    }

    fn live(&self, reported: &str, pane: Option<&str>) -> String {
        live_session(&self.files, &self.state_dir, reported, pane)
    }

    fn marks(&self, tab: Option<&str>) -> Result<Vec<String>, Failure> {
        // Python's _base_label opens a store of its own for this one read
        let store = open(&self.db)?;
        marks_for(store.db(), tab).map_err(failure)
    }
}

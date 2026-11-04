//! What `inbox radar-sidebar` did, and what it says (Python
//! `cli.RADAR_MESSAGES`).

use std::path::Path;

/// The outcome of [`super::ensure`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The rows were edited and herdr reloaded.
    Changed,
    /// They are as asked already.
    Unchanged,
    /// No herdr config.
    NoConfig,
    /// herdr-radar is not installed (or disabled).
    NoRadar,
    /// No radar block (or more than one, or its markers out of order).
    NoBlock,
    /// The file does not parse.
    Invalid,
    /// Radar carries `$inbox_q` itself.
    SteppedAside,
    /// TOML in the block this editor does not walk.
    Unsupported,
    /// The file changed while editing.
    Conflict,
}

impl Status {
    /// Its line, word for word Python's (`path`: herdr's config).
    pub fn message(self, path: &Path, remove: bool) -> String {
        let path = path.display();
        let verb = if remove { "removed from" } else { "added to" };
        match self {
            Status::Changed => format!(
                "$inbox_q {verb} herdr-radar's sidebar rows in {path} (backup: config.toml.inbox-bak); herdr reloaded"
            ),
            Status::Unchanged => {
                format!(
                    "nothing to change: herdr-radar's sidebar rows in {path} are already as asked"
                )
            }
            Status::NoConfig => format!("no herdr config at {path}"),
            Status::NoRadar => {
                "herdr-radar is not installed (or disabled): nothing to do".to_string()
            }
            Status::NoBlock => format!("no herdr-radar sidebar block in {path}: nothing to do"),
            Status::Invalid => format!("{path} does not parse as TOML: left untouched"),
            Status::SteppedAside => {
                "herdr-radar already carries $inbox_q (extra_tokens): left to radar".to_string()
            }
            Status::Unsupported => format!(
                "herdr-radar's block in {path} uses TOML this command does not edit ([[tables]] or multi-line strings): left untouched"
            ),
            Status::Conflict => {
                format!("{path} changed while editing: left untouched, run it again")
            }
        }
    }

    /// The command's exit code: 0 when the rows are as asked.
    pub fn code(self) -> i32 {
        match self {
            Status::Changed | Status::Unchanged | Status::SteppedAside => 0,
            _ => 1,
        }
    }
}

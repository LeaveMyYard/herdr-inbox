//! A session and the live sessions that continued into it (Python
//! `store.family`): whose answers a session's hooks may carry.
//!
//! `sessions.continued_in` (written by the worker) only nominates a
//! predecessor. It counts while its transcript STILL continues into this
//! session (the D6 rule): a resumed old session owns its rows again.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::sessions::session_transcript;
use crate::claude::{Transcripts, continued_in};

/// Sessions in one family at most (cycle-safe either way).
pub const FAMILY_LIMIT: usize = 32;

/// What [`family`] asks of the transcripts, so it can be tested without
/// Claude Code's files.
pub trait Continuations {
    /// The transcript of `session_id` (`known`: the recorded path).
    fn find(&self, session_id: &str, known: Option<&str>) -> Option<PathBuf>;
    /// The canonical id of the session that transcript continued in.
    fn continued_in(&self, path: &Path) -> Option<String>;
}

impl Continuations for Transcripts {
    fn find(&self, session_id: &str, known: Option<&str>) -> Option<PathBuf> {
        Transcripts::find(self, session_id, known)
    }

    fn continued_in(&self, path: &Path) -> Option<String> {
        continued_in(path).map(|s| s.as_str().to_owned())
    }
}

/// `session_id` first, then each predecessor whose transcript continues into
/// a member, depth first, at most [`FAMILY_LIMIT`]; empty for an empty id.
/// The ids are compared as stored: a predecessor's transcript must name the
/// member exactly (Python compares the canonical id to the stored one).
pub fn family(
    db: &Connection,
    session_id: &str,
    files: &impl Continuations,
) -> rusqlite::Result<Vec<String>> {
    if session_id.is_empty() {
        return Ok(Vec::new());
    }
    let mut fam = vec![session_id.to_owned()];
    let mut frontier = vec![session_id.to_owned()];
    let mut stmt = db.prepare("SELECT session_id FROM sessions WHERE continued_in = ?")?;
    while fam.len() < FAMILY_LIMIT {
        let Some(current) = frontier.pop() else { break };
        let prevs = stmt
            .query_map([&current], |r| r.get::<_, Option<String>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for prev in prevs.into_iter().flatten() {
            if fam.contains(&prev) {
                continue;
            }
            let known = session_transcript(db, &prev)?;
            let Some(path) = files.find(&prev, known.as_deref()) else {
                continue;
            };
            if files.continued_in(&path).as_deref() == Some(current.as_str()) {
                fam.push(prev.clone());
                frontier.push(prev);
            }
        }
    }
    Ok(fam)
}

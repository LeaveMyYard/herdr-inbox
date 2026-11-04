//! Where a typed answer counts (Python `service.question_family`,
//! `older_conversation`, `_watch_paths`, `_verify_plans`): the transcripts
//! watched while it lands, and per answer the ones a landing in delivers it
//! (its own forward family, decided per answer, never for the batch) or
//! the older-conversation state when it lands only in an older session.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use herdr_agent_kit::agent_session::SessionId;
use inbox_core::deliver::{Agent, older_error};
use inbox_core::verify::Landing;
use rusqlite::Connection;

use super::forward_family;
use super::watch::{Watched, size};
use crate::claude::Transcripts;
use crate::store::Question;
use crate::store::sessions::session_transcript;

/// The sessions `q`'s answer may go to or be verified in: its asker's
/// forward family, bounded by when it was asked.
pub fn question_family(
    db: &Connection,
    files: &Transcripts,
    q: &Question,
) -> rusqlite::Result<Vec<String>> {
    forward_family(db, files, q.session_id.as_deref(), Some(&q.created_at))
}

/// `live` (what a pane runs) is an older session of `asker`'s conversation:
/// it continued, perhaps transitively, into the asker, and is not itself in
/// the asker's forward family.
pub fn older_conversation(
    db: &Connection,
    files: &Transcripts,
    live: Option<&str>,
    asker: Option<&str>,
) -> rusqlite::Result<bool> {
    let (Some(live), Some(asker)) = (
        live.filter(|s| !s.is_empty()),
        asker.filter(|s| !s.is_empty()),
    ) else {
        return Ok(false);
    };
    if live == asker {
        return Ok(false);
    }
    Ok(forward_family(db, files, Some(live), None)?
        .iter()
        .any(|s| s == asker)
        && !forward_family(db, files, Some(asker), None)?
            .iter()
            .any(|s| s == live))
}

/// The transcripts of the pane's followed and reported sessions, each from
/// its current size (`fresh`: from 0, for a file that appeared with the
/// typing).
pub fn watch_paths(
    db: &Connection,
    files: &Transcripts,
    agent: &Agent,
    fresh: bool,
) -> rusqlite::Result<Vec<Watched>> {
    let mut out: Vec<Watched> = Vec::new();
    for s in [agent.session.as_deref(), agent.reported_session.as_deref()]
        .into_iter()
        .flatten()
    {
        if s.is_empty() {
            continue;
        }
        let known = session_transcript(db, s)?;
        if let Some(p) = files.find(s, known.as_deref())
            && !out.iter().any(|(seen, _)| *seen == p)
        {
            let offset = if fresh { 0 } else { size(&p) };
            out.push((p, offset));
        }
    }
    Ok(out)
}

/// Where one answer of a typed batch counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The watched transcripts a landing in delivers it.
    pub counting: Vec<PathBuf>,
    /// Its state if the text lands only in an older session of its conversation.
    pub older_error: Option<String>,
}

impl Plan {
    /// Where the text landed (`hits`), as this answer's landing.
    pub fn landing(&self, hits: &BTreeSet<PathBuf>) -> Landing {
        Landing {
            counted: self.counting.iter().any(|p| hits.contains(p)),
            anywhere: !hits.is_empty(),
            older_error: self.older_error.clone(),
        }
    }
}

fn stem_session(path: &Path) -> Option<SessionId> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(SessionId::parse)
}

/// Per answer of `batch`: its plan over the watched transcripts.
pub fn verify_plans(
    db: &Connection,
    files: &Transcripts,
    pane: &str,
    watch: &[Watched],
    batch: &[&Question],
) -> rusqlite::Result<BTreeMap<i64, Plan>> {
    let mut plans = BTreeMap::new();
    for q in batch {
        let family = question_family(db, files, q)?;
        let asker = q.session_id.as_deref().filter(|s| !s.is_empty());
        let mut plan = Plan::default();
        for (p, _) in watch {
            let sid = stem_session(p);
            match (&sid, asker) {
                (Some(s), Some(_)) if !family.iter().any(|f| f == s.as_str()) => {
                    if plan.older_error.is_none()
                        && older_conversation(db, files, Some(s.as_str()), asker)?
                    {
                        plan.older_error = Some(older_error(pane, Some(s.as_str()), asker, true));
                    }
                }
                _ => plan.counting.push(p.clone()),
            }
        }
        plans.insert(q.id, plan);
    }
    Ok(plans)
}

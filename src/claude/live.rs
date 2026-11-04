//! Which session a Claude process runs now (Python `store.live_session`).
//!
//! herdr (and the process's own `CLAUDE_CODE_SESSION_ID`) keep reporting the
//! id a conversation started under. After a continuation the conversation
//! lives in the session the transcript's `continued-in` names (D6), unless a
//! Claude process (re)started that id AFTER its continued-in record: herdr
//! resumes a pane under the OLD id, and until its first prompt the old
//! transcript still ends in continued-in (live 2026-10-17: synthetic_100–synthetic_98 typed
//! into synthetic_89, an ancestor of the asking synthetic_90). The SessionStart
//! hook's resume marker (`starts`) says when.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use herdr_agent_kit::agent_session::SessionId;

use super::{Transcripts, chain, continuation};

/// The session `reported` runs now. `pane`: the herdr pane asked about, when
/// known; a marker written by a resume in ANOTHER pane never applies here
/// (one pane can run OLD's in-process continuation while another runs OLD
/// resumed). `state_dir`: the inbox's state directory (`starts/` below it).
pub fn live_session(
    files: &Transcripts,
    state_dir: &Path,
    reported: &str,
    pane: Option<&str>,
) -> String {
    live_session_known(files, state_dir, reported, None, pane)
}

/// [`live_session`] with `reported`'s recorded transcript path, `known`
/// (Python `live_session(reported, known, pane)`): a transcript the
/// projects folder no longer lists is still found there.
pub fn live_session_known(
    files: &Transcripts,
    state_dir: &Path,
    reported: &str,
    known: Option<&str>,
    pane: Option<&str>,
) -> String {
    if reported.is_empty() {
        return String::new();
    }
    if let Some(path) = files.find(reported, known)
        && let Some((_, Some(at))) = continuation(&path)
        && let Some(sid) = SessionId::parse(reported)
    {
        let marker = state_dir.join("starts").join(sid.as_str());
        let started = fs::metadata(&marker)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64());
        let marked = fs::read_to_string(&marker)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let other_pane =
            matches!((pane, marked.as_deref()), (Some(p), Some(m)) if !p.is_empty() && m != p);
        if started.is_some_and(|s| s > at) && !other_pane {
            return sid.as_str().to_owned();
        }
    }
    chain(files, reported, known)
        .pop()
        .unwrap_or_else(|| reported.to_owned())
}

#[cfg(test)]
mod tests;

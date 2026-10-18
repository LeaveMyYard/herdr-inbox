//! SessionStart (Python `hook.session_start_context`, `_remember_transcript`
//! and the start marker in `hook.run`).

use std::path::Path;

use super::deliver::RECORD_TIMEOUT;
use super::payload::Payload;
use super::rules::{PENDING_HEADING, SESSION_RULES};
use crate::starts::record_start;
use crate::store::Store;
use crate::store::pending::pending_lines;
use crate::store::sessions::record_session;

/// First and cheap: a RESUMED session id runs itself from now on, not the
/// session its transcript once continued in (`store.live_session` reads the
/// marker). Only a resume re-runs an id that may have continued elsewhere: a
/// compaction continuation fires `compact`, and clear/fork/startup run a
/// fresh id. Errors are ignored.
pub fn mark_resume(state_dir: &Path, payload: &Payload, pane: Option<&str>) {
    if payload.source.as_deref() == Some("resume")
        && let Some(session) = payload.session_id.as_deref()
    {
        let _ = record_start(state_dir, session, pane);
    }
}

/// The rules, then, after a compaction or a resume, the session's questions
/// still with the operator.
pub fn context(db_path: &Path, payload: &Payload) -> String {
    let mut text = SESSION_RULES.to_owned();
    let listed = matches!(payload.source.as_deref(), Some("compact" | "resume"));
    if let (true, Some(session)) = (listed, payload.session_id.as_deref()) {
        let lines = pending_lines(db_path, session);
        if !lines.is_empty() {
            text.push_str(PENDING_HEADING);
            text.push_str(&lines.join("\n"));
        }
    }
    text
}

/// After the rules are out: record where the transcript is (verification
/// falls back to the projects directory without it, so a locked, missing or
/// not yet migrated database costs nothing but this).
pub fn remember_transcript(db_path: &Path, payload: &Payload) {
    let (Some(session), Some(path)) = (
        payload.session_id.as_deref(),
        payload.transcript_path.as_deref(),
    ) else {
        return;
    };
    if let Ok(store) = Store::open(db_path, RECORD_TIMEOUT) {
        let _ = record_session(store.db(), session, Some(path));
    }
}

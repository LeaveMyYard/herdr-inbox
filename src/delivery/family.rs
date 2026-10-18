//! The asker's forward family (Python `service.forward_family`, herdr-inbox
//! #53): the sessions an answer may go to or be verified in.

use herdr_agent_kit::agent_session::claude::wire::ts;
use rusqlite::Connection;
use serde_json::Value;

use crate::claude::{Transcripts, chain, continued_edges};
use crate::store::family::FAMILY_LIMIT;
use crate::store::sessions::session_transcript;

/// `session` and every session its conversation continued into,
/// transitively: its current D6 chain plus the sessions its transcript EVER
/// continued in. Never an ANCESTOR (a session that continued into it): that
/// is an older conversation without the asker's context (live 2026-10-17:
/// Q129–Q152 counted as delivered in an ancestor).
///
/// `since` (the question's `created_at`): a past continuation counts only if
/// its record is dated at or after it. An undated past record never counts;
/// the current chain always does. At most [`FAMILY_LIMIT`] sessions.
pub fn forward_family(
    db: &Connection,
    files: &Transcripts,
    session: Option<&str>,
    since: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    let Some(session) = session.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let known = session_transcript(db, session)?;
    let mut out = chain(files, session, known.as_deref());
    let bound = since
        .filter(|s| !s.is_empty())
        .and_then(|s| ts(&Value::String(s.to_owned())))
        .map(|t| t.secs());
    let mut i = 0;
    while i < out.len() && out.len() < FAMILY_LIMIT {
        let known = session_transcript(db, &out[i])?;
        if let Some(path) = files.find(&out[i], known.as_deref()) {
            for edge in continued_edges(&path) {
                let dated_after = matches!((edge.at, bound), (Some(at), Some(b)) if at >= b);
                if !out.contains(&edge.target) && (since.is_none() || dated_after) {
                    out.push(edge.target);
                }
            }
        }
        i += 1;
    }
    Ok(out)
}

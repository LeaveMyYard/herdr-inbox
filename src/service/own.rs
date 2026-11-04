//! Whether this process runs in the conversation that asked (Python
//! `service.answered_by_own_conversation`): an agent answering its own
//! question with the CLI, e.g. relaying the operator's decision from chat.
//! **SAFETY**: such an answer is never typed; a wrong yes loses a delivery.

use std::path::Path;

use crate::claude::Transcripts;
use crate::claude::live::live_session_known;
use crate::delivery::plans::question_family;
use crate::store::sessions::session_transcript;
use crate::store::{Question, Store};

/// Who this process is: its Claude Code session (`CLAUDE_CODE_SESSION_ID`,
/// which only a Claude Code session's own processes have; the operator's
/// shell, even in a herdr pane, does not) and its herdr pane.
#[derive(Clone, Copy, Debug, Default)]
pub struct Me<'a> {
    /// `CLAUDE_CODE_SESSION_ID`.
    pub session: Option<&'a str>,
    /// `HERDR_PANE_ID`.
    pub pane: Option<&'a str>,
}

/// `me` runs in the conversation that asked `q`: the session it runs now
/// (its continuations followed) is in `q`'s forward family, the asking
/// session or one it continued into, never an older one.
pub fn answered_by_own_conversation(
    store: &Store,
    files: &Transcripts,
    state_dir: &Path,
    me: Me<'_>,
    q: &Question,
) -> rusqlite::Result<bool> {
    let (Some(_), Some(session)) = (
        q.session_id.as_deref().filter(|s| !s.is_empty()),
        me.session.filter(|s| !s.is_empty()),
    ) else {
        return Ok(false);
    };
    let known = session_transcript(store.db(), session)?;
    let pane = me.pane.filter(|p| !p.is_empty());
    let live = live_session_known(files, state_dir, session, known.as_deref(), pane);
    Ok(question_family(store.db(), files, q)?.contains(&live))
}

#[cfg(test)]
mod tests;

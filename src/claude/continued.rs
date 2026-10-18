//! Whether a transcript's conversation continued in another session (Python
//! `transcript.continued_in`, the D6 rule), read the way a hook reads it: one
//! cold look at the file's tail.
//!
//! The rule itself is the kit's ([`Continuation`]): a `continued-in` record
//! in the main transcript marks the session continued until a later `user` or
//! `assistant` record. Python's hook process starts with an empty cache, so it
//! reads only the last [`FOLLOW_TAIL`] bytes, up to the last complete line;
//! a first line cut by the window does not parse and counts for nothing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use herdr_agent_kit::agent_session::SessionId;
use herdr_agent_kit::agent_session::claude::continuation::Continuation;

/// The bytes a cold look reads from the end of the transcript (Python
/// `transcript.FOLLOW_TAIL`).
pub const FOLLOW_TAIL: u64 = 64 * 1024;

/// A session id no real session has: the owner of a transcript whose file
/// name is not a session id (a recorded path), so that no target equals it,
/// as Python's `None` never did.
const NO_OWNER: &str = "00000000-0000-0000-0000-000000000000";

/// The session `path`'s conversation continued in, or `None`: never
/// continued, resumed since, unreadable, or a subagent's own transcript
/// (only the main one moves the conversation).
pub fn continued_in(path: &Path) -> Option<SessionId> {
    if in_subagents_dir(path) {
        return None;
    }
    let data = tail(path)?;
    let end = data.iter().rposition(|&b| b == b'\n')?;
    let own = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(SessionId::parse)
        .or_else(|| SessionId::parse(NO_OWNER))?;
    let mut state = Continuation::new(own);
    let lines: Vec<&[u8]> = data[..end].split(|&b| b == b'\n').collect();
    state.feed(&lines);
    state.next().cloned()
}

/// The last [`FOLLOW_TAIL`] bytes of the file.
fn tail(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(FOLLOW_TAIL)))
        .ok()?;
    let mut data = Vec::new();
    file.take(FOLLOW_TAIL).read_to_end(&mut data).ok()?;
    Some(data)
}

/// `subagents` is one of the three directories above the file (Python:
/// `"subagents" in path.parts[-4:-1]`; folders above the session never count).
fn in_subagents_dir(path: &Path) -> bool {
    path.parent().is_some_and(|dir| {
        dir.components()
            .rev()
            .take(3)
            .any(|c| c.as_os_str() == "subagents")
    })
}

#[cfg(test)]
mod tests;

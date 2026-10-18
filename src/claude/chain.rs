//! A conversation's sessions in order (Python `transcript.chain`): the D6
//! continuation followed from a session, as a hook process sees it.

use herdr_agent_kit::agent_session::SessionId;

use super::{Transcripts, continued_in};

/// Hops followed at most (Python's `max_hops` default).
pub const MAX_HOPS: usize = 8;

/// `session_id` (canonical when it is a session id, else as given) and each
/// session its conversation continued in, in order, cycle-safe and at most
/// [`MAX_HOPS`]. `known` is the first session's recorded transcript path.
pub fn chain(files: &Transcripts, session_id: &str, known: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = Some(
        SessionId::parse(session_id)
            .map(|s| s.as_str().to_owned())
            .unwrap_or_else(|| session_id.to_owned()),
    );
    for hop in 0..MAX_HOPS {
        let Some(id) = current.take().filter(|c| !c.is_empty() && !out.contains(c)) else {
            break;
        };
        let path = files.find(&id, if hop == 0 { known } else { None });
        out.push(id);
        current = path
            .as_deref()
            .and_then(continued_in)
            .map(|s| s.as_str().to_owned());
    }
    out
}

#[cfg(test)]
mod tests;

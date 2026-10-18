//! The delivery hook's policy (Python `hook.deliver`): a session's waiting
//! answers handed to it by the session's own hook, or by its rewake watcher.
//! Today's only caller is the Claude Code hook (`herdr-inbox hook`).
//!
//! The order of the steps, each deciding before the next lookup is made:
//!
//! 1. [`gate`]: the session's own hook, an answer waiting (read-only), a
//!    usable schema; any one failing means nothing to do.
//! 2. Per candidate, once claimed and checked against every transcript of its
//!    conversation: [`carry`].
//! 3. [`message`]: what is handed over, from what was recorded as sent.

use std::collections::BTreeSet;

use crate::REFERENCE_INDEX;

/// Does this firing belong to the session that asked? A session id is known
/// and it is not a subagent's or a teammate's hook. `agent_id`: the payload's
/// `agent_id` is present and truthy as Python reads it (`""`, `null`, `0`
/// count as none).
pub fn own_session(session_id: Option<&str>, agent_id: bool) -> bool {
    session_id.is_some_and(|s| !s.is_empty()) && !agent_id
}

/// Where the hook stopped before any claim, or that it goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Not the asking session's own hook.
    NotOwnSession,
    /// No answer of the session's family is waiting (read-only check).
    NothingWaiting,
    /// The database is not migrated to a schema this code can use.
    SchemaNotReady,
    /// Claim and carry the candidates.
    Carry,
}

/// The gates in order; each lookup runs only when the gates before it passed.
pub fn gate(
    own_session: bool,
    has_candidates: impl FnOnce() -> bool,
    schema_ready: impl FnOnce() -> bool,
) -> Gate {
    if !own_session {
        Gate::NotOwnSession
    } else if !has_candidates() {
        Gate::NothingWaiting
    } else if !schema_ready() {
        Gate::SchemaNotReady
    } else {
        Gate::Carry
    }
}

/// What became of one candidate answer at this firing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carry {
    /// Another process holds its claim: left alone.
    NotClaimed,
    /// Already in the conversation (a late landing): recorded delivered and
    /// verified, never sent twice. `acknowledge`: more than the long-answer
    /// reference landed.
    Landed {
        /// Record it as seen by the agent.
        acknowledge: bool,
    },
    /// Not in this firing's message (it is cut at herdr's argument cap): the
    /// claim is given back untouched and the next firing carries it.
    NextFiring,
    /// In this firing's message: recorded hook-sent under its claim, before
    /// the text is printed (the worker verifies it later).
    Send,
}

/// What the hook knows about one candidate when it decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// This firing holds its delivery claim.
    pub claimed: bool,
    /// The indexes of its expected strings already in the conversation
    /// (`None`: not landed).
    pub landed: Option<&'a BTreeSet<usize>>,
    /// It is in the first message of the not-yet-landed candidates.
    pub in_this_message: bool,
}

/// The step for one candidate (see [`Carry`]).
pub fn carry(c: &Candidate<'_>) -> Carry {
    if !c.claimed {
        return Carry::NotClaimed;
    }
    if let Some(kinds) = c.landed {
        let acknowledge = kinds.iter().any(|&k| k != REFERENCE_INDEX);
        return Carry::Landed { acknowledge };
    }
    if c.in_this_message {
        Carry::Send
    } else {
        Carry::NextFiring
    }
}

/// What the hook prints after recording the sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// Nothing was recorded as sent (claims lost meanwhile): no output.
    Nothing,
    /// Every answer of the message was recorded: print it.
    AsBuilt,
    /// Only some were: print a message rebuilt from those.
    Rebuilt,
}

/// The output for `sent` of the `in_message` answers recorded hook-sent.
pub fn message(sent: usize, in_message: usize) -> Message {
    match sent {
        0 => Message::Nothing,
        n if n == in_message => Message::AsBuilt,
        _ => Message::Rebuilt,
    }
}

#[cfg(test)]
mod tests;

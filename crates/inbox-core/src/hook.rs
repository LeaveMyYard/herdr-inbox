//! The delivery hook's policy (Python `hook.deliver`): a session's waiting
//! answers handed to it as hook context at its next tool batch or turn end,
//! or by its rewake watcher.
//!
//! The order of the steps, each deciding before the next lookup is made:
//!
//! 1. [`own_session`]: only the session's own hook carries its answers.
//! 2. The candidates check (read-only), then the schema check: either one
//!    failing means nothing to do.
//! 3. Per candidate, once claimed and checked against every transcript of its
//!    conversation: [`carry`].
//! 4. [`message`]: what is printed, from what was recorded as sent.

use std::collections::BTreeSet;

use crate::REFERENCE_INDEX;

/// Does this firing belong to the session that asked? Not a subagent's or a
/// teammate's hook (the payload names an `agent_id`), and a session id is
/// known.
pub fn own_session(session_id: Option<&str>, agent_id: bool) -> bool {
    session_id.is_some_and(|s| !s.is_empty()) && !agent_id
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

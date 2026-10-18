//! The rewake watcher's policy (Python `rewake.run`, `rewake._ready`): when a
//! watcher for an idle session starts, looks at its pane, delivers or ends.
//!
//! A watcher polls for an answer its session has not seen. While herdr
//! reports the pane's agent at rest with this very session, it delivers the
//! answers (the caller then wakes the session); a working session gets them
//! another way, and a blocked one is not disturbed. Today's only caller is
//! the Claude Code asyncRewake hook (`herdr-inbox hook rewake-watch`).
//!
//! [`Watch`] holds the poll's clocks; times are seconds on a monotonic clock
//! the caller reads.

/// Seconds between herdr looks even with nothing pending: the pane switching
/// conversation ends the watch (#30 M1).
pub const PANE_CHECK_EVERY: f64 = 45.0;
/// herdr names no session for the pane this long: the watch ends (#30 M1b).
pub const UNKNOWN_SESSION_END: f64 = 180.0;
/// The watch ends after this (under the 12 h its caller may run for).
pub const WATCH_MAX_SECONDS: f64 = 12.0 * 3600.0 - 300.0;
/// Seconds a new watcher waits for the one it supersedes to let go.
pub const LOCK_WAIT: f64 = 5.0;

/// Does a watcher start at all? Only for a valid session id, in a herdr
/// pane, and not for a subagent's or teammate's firing (`agent_id` present
/// and truthy, as Python reads it).
pub fn start(valid_session_id: bool, in_pane: bool, agent_id: bool) -> bool {
    valid_session_id && in_pane && !agent_id
}

/// While waiting for the superseded watcher's lock: try again, or give up
/// (the wait is over, or a newer watcher took ownership meanwhile).
pub fn keep_waiting_for_lock(started: f64, now: f64, still_owner: bool) -> bool {
    now < started + LOCK_WAIT && still_owner
}

/// The pane agent's status, as herdr reports it (the caller maps herdr's
/// strings). TODO(kit AgentStatus): use the kit's neutral status type once
/// it carries `done`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// `idle`.
    Idle,
    /// `done`.
    Done,
    /// `working`.
    Working,
    /// `blocked`.
    Blocked,
    /// Anything else, or none reported.
    Other,
}

/// The pane, as herdr reports it, for this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ready {
    /// At rest (idle or done) with this very session: may be delivered to.
    Go,
    /// Working, blocked or another status: not now.
    Wait,
    /// herdr names no session for the pane: never a delivery (#30 M1b).
    Unknown,
    /// The agent is gone, or the pane holds another conversation now.
    End,
}

impl Ready {
    /// The name the decision table uses.
    pub fn name(self) -> &'static str {
        match self {
            Ready::Go => "go",
            Ready::Wait => "wait",
            Ready::Unknown => "unknown",
            Ready::End => "end",
        }
    }
}

/// The pane's agent: the session it runs (the live one, continuations
/// followed) and its status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneAgent<'a> {
    /// The live session, if herdr names one.
    pub session_id: Option<&'a str>,
    /// Its status.
    pub status: Status,
}

/// [`Ready`] for `session_id` from the pane's agent (`None`: no agent).
pub fn ready(agent: Option<PaneAgent<'_>>, session_id: &str) -> Ready {
    let Some(agent) = agent else {
        return Ready::End;
    };
    match agent.session_id.filter(|s| !s.is_empty()) {
        None => Ready::Unknown,
        Some(s) if s != session_id => Ready::End,
        Some(_) if matches!(agent.status, Status::Idle | Status::Done) => Ready::Go,
        Some(_) => Ready::Wait,
    }
}

/// What a poll decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The watch is over: exit without waking.
    End,
    /// Nothing to do now: poll again later.
    KeepWatching,
    /// Pending answers and the pane at rest with this session: run the
    /// delivery hook's path now.
    Deliver,
    /// The delivery produced text: hand it over and wake the session.
    Wake,
}

impl Verdict {
    /// The name the decision table uses (`Deliver` is not an outcome).
    pub fn name(self) -> &'static str {
        match self {
            Verdict::End => "end",
            Verdict::KeepWatching => "keep watching",
            Verdict::Deliver => "deliver",
            Verdict::Wake => "wake",
        }
    }
}

/// One watcher's poll state: its deadline, when herdr was last asked, and
/// since when herdr names no session for the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Watch {
    end: f64,
    last_check: f64,
    unknown_since: Option<f64>,
}

impl Watch {
    /// A watch started at `now` (once it holds its lock), ending at the
    /// latest [`WATCH_MAX_SECONDS`] later.
    pub fn new(now: f64) -> Watch {
        Watch {
            end: now + WATCH_MAX_SECONDS,
            last_check: now,
            unknown_since: None,
        }
    }

    /// The first check of every poll: the watch's time is up, a newer
    /// watcher took over (`still_owner` false), or the agent's process is
    /// gone (`alive` false).
    pub fn begin_poll(&self, now: f64, still_owner: bool, alive: bool) -> Verdict {
        if now >= self.end || !still_owner || !alive {
            Verdict::End
        } else {
            Verdict::KeepWatching
        }
    }

    /// Ask herdr now? With answers pending, or every [`PANE_CHECK_EVERY`]
    /// even without. Asking resets the herdr clock.
    pub fn look(&mut self, pending: bool, now: f64) -> bool {
        let due = pending || now - self.last_check >= PANE_CHECK_EVERY;
        if due {
            self.last_check = now;
        }
        due
    }

    /// The verdict once herdr answered [`Ready`].
    pub fn after_look(&mut self, pending: bool, ready: Ready, now: f64) -> Verdict {
        match ready {
            Ready::End => return Verdict::End,
            Ready::Unknown => {
                let since = *self.unknown_since.get_or_insert(now);
                if now - since >= UNKNOWN_SESSION_END {
                    return Verdict::End;
                }
            }
            Ready::Go | Ready::Wait => self.unknown_since = None,
        }
        if ready == Ready::Go && pending {
            Verdict::Deliver
        } else {
            Verdict::KeepWatching
        }
    }

    /// The verdict after a [`Verdict::Deliver`]: the delivery produced text
    /// (`hook_sent`) or not (claims lost, landed meanwhile).
    pub fn after_delivery(hook_sent: bool) -> Verdict {
        if hook_sent {
            Verdict::Wake
        } else {
            Verdict::KeepWatching
        }
    }
}

#[cfg(test)]
mod tests;

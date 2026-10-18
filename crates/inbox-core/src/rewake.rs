//! The asyncRewake watcher's policy (Python `rewake.run`, `rewake._ready`).
//!
//! Claude Code runs the watcher in the background after every `Stop`. It
//! polls for an answer this session has not seen; while herdr reports the
//! pane's agent at rest with this very session it delivers the answers as
//! hook text, which the launcher turns into the wake exit. A working session
//! gets them from its PostToolBatch/Stop hooks instead; a blocked one is not
//! disturbed.
//!
//! [`Watch`] holds the poll's two clocks; times are seconds on a monotonic
//! clock the caller reads.

/// Seconds between database looks (read-only).
pub const WATCH_POLL: f64 = 2.0;
/// Seconds between herdr looks even with nothing pending: a `/clear` or
/// `/resume` in the pane ends the watch (#30 M1).
pub const PANE_CHECK_EVERY: f64 = 45.0;
/// herdr names no session for the pane this long: the watch ends (#30 M1b).
pub const UNKNOWN_SESSION_END: f64 = 180.0;
/// The watch ends after this, under the hooks.json timeout (43200 s) that
/// Claude Code enforces.
pub const WATCH_MAX_SECONDS: f64 = 12.0 * 3600.0 - 300.0;
/// Seconds to wait for a superseded watcher to let go of the lock.
pub const LOCK_WAIT: f64 = 5.0;
/// The watcher's private "wake" exit code; the hooks.json wrapper maps it,
/// and only it, to Claude Code's exit 2.
pub const WAKE_EXIT: i32 = 3;

/// The pane, as herdr reports it, for this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ready {
    /// At rest (`idle` or `done`) with this very session: may be woken.
    Go,
    /// Working, blocked or an unknown status: not now.
    Wait,
    /// herdr names no session for the pane: never a wake (#30 M1b).
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

/// The pane's agent as herdr reports it: the session it runs (the live one,
/// continuations followed) and its status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneAgent<'a> {
    /// The live session, if herdr names one.
    pub session_id: Option<&'a str>,
    /// herdr's status (`idle`, `done`, `working`, `blocked`, ...).
    pub status: Option<&'a str>,
}

/// [`Ready`] for `session_id` from the pane's agent (`None`: no agent).
pub fn ready(agent: Option<PaneAgent<'_>>, session_id: &str) -> Ready {
    let Some(agent) = agent else {
        return Ready::End;
    };
    match agent.session_id.filter(|s| !s.is_empty()) {
        None => Ready::Unknown,
        Some(s) if s != session_id => Ready::End,
        Some(_) if matches!(agent.status, Some("idle" | "done")) => Ready::Go,
        Some(_) => Ready::Wait,
    }
}

/// What a poll decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The watch is over: exit without a wake.
    End,
    /// Nothing to do now: stamp liveness, sleep [`WATCH_POLL`], poll again.
    KeepWatching,
    /// Pending answers and the pane at rest with this session: run the
    /// delivery hook's path now.
    Deliver,
    /// The delivery produced text: print it and exit [`WAKE_EXIT`].
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

/// One watcher's poll state: when herdr was last asked, and since when it
/// names no session for the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Watch {
    last_check: f64,
    unknown_since: Option<f64>,
}

impl Watch {
    /// A watch started at `now`.
    pub fn new(now: f64) -> Watch {
        Watch {
            last_check: now,
            unknown_since: None,
        }
    }

    /// The first check of every poll: a newer watcher took over
    /// (`still_owner` false) or the Claude process is gone (`alive` false).
    pub fn owner(still_owner: bool, alive: bool) -> Verdict {
        if still_owner && alive {
            Verdict::KeepWatching
        } else {
            Verdict::End
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

    /// The verdict after a [`Verdict::Deliver`]: the hook path produced text
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

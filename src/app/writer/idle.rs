//! When the writer leaves: `herdr-app status` for this app home, every 2 s and
//! bounded to 500 ms. Exit 0 means the app runs, exit 2 is a confirmed absent
//! window, and anything else says nothing, so it restarts the count.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How often the app is asked about.
pub const EVERY: Duration = Duration::from_secs(2);
/// The longest one question takes.
pub const BOUND: Duration = Duration::from_millis(500);
/// Confirmed absence for this long: the writer exits.
pub const ABSENT_EXIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(10);

/// What the app's status says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// A window runs.
    Running,
    /// No window.
    Absent,
    /// Nothing proven.
    Unknown,
}

/// Asks `herdr-app` about app home `home`. Only the child started here is ever
/// killed, when it outlasts [`BOUND`].
pub fn probe(home: &Path) -> State {
    let spawned = Command::new("herdr-app")
        .arg("status")
        .env("HERDR_APP_HOME", home)
        .env_remove("HC_TERMINAL_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        return State::Unknown;
    };
    let deadline = Instant::now() + BOUND;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return state(status.code()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return State::Unknown;
            }
        }
    }
}

/// What an exit of `herdr-app status` says.
fn state(code: Option<i32>) -> State {
    match code {
        Some(0) => State::Running,
        Some(2) => State::Absent,
        _ => State::Unknown,
    }
}

/// Counts confirmed absence.
#[derive(Debug, Default)]
pub struct Idle {
    since: Option<Duration>,
}

impl Idle {
    /// Takes `state` seen at `now`, a time since any fixed start. True when
    /// the writer should leave.
    pub fn observe(&mut self, state: State, now: Duration) -> bool {
        if state != State::Absent {
            self.since = None;
            return false;
        }
        now.saturating_sub(*self.since.get_or_insert(now)) >= ABSENT_EXIT
    }
}

#[cfg(test)]
mod tests;

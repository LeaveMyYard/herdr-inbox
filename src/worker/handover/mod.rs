//! Handing the worker lock to a newer revision (Python `worker_handover.py`,
//! herdr-inbox #50): the holder and standby records beside the lock, byte
//! for byte Python's, so a Python and a Rust worker hand over to each other.
//!
//! - **Holder** `<lock>.holder`: `<revision> <pid> <start>`, written by the
//!   lock's holder, removed when it lets go. A worker from before the
//!   record (0.10.6 and older) is revision 1.
//! - **Standby** `<lock>.standby`: `<revision> <pid> <unix seconds> <start>`,
//!   rewritten every [`STANDBY_HEARTBEAT`] by a newer worker waiting for the
//!   lock; it retries the lock every [`STANDBY_POLL`] for at most
//!   [`STANDBY_MAX`].
//!
//! `<start>` is the process's start time as the OS reports it ([`start_time`],
//! `-` where it cannot be read): a reused pid never passes for the holder.
//! Handover is cooperative only: nothing here signals another process; the
//! holder releases the lock itself when it sees a newer standby.

mod proc_start;
mod records;
mod standby;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use proc_start::start_time;
pub use records::{clear_holder, holder_revision, live_standby, write_holder};
pub use standby::{Standby, wait_as_standby};

/// How often a standby retries the lock.
pub const STANDBY_POLL: Duration = Duration::from_millis(100);
/// How often a standby rewrites its record.
pub const STANDBY_HEARTBEAT: Duration = Duration::from_secs(1);
/// A heartbeat older (or further ahead) than this many seconds: the standby
/// is gone.
pub const STANDBY_FRESH: f64 = 10.0;
/// How long a standby waits at most.
pub const STANDBY_MAX: Duration = Duration::from_secs(1800);
/// The holder starts a successor for a revision-wanted marker at most this
/// often.
pub const SPAWN_EVERY: Duration = Duration::from_secs(60);

/// `<lock>.holder`.
pub fn holder_path(lock: &Path) -> PathBuf {
    suffixed(lock, ".holder")
}

/// `<lock>.standby`.
pub fn standby_path(lock: &Path) -> PathBuf {
    suffixed(lock, ".standby")
}

fn suffixed(lock: &Path, suffix: &str) -> PathBuf {
    let mut name = lock.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The record at `path` names this process (its second word is our pid).
fn names_me(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|t| t.split_whitespace().nth(1) == Some(std::process::id().to_string().as_str()))
}

#[cfg(test)]
mod tests;

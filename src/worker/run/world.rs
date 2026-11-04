//! [`World`]: everything the run loop asks of the world besides its lock and
//! records, so the loop's rules are tested without processes or herdr.
//! [`crate::worker::live::Live`] is the real one.

use std::path::PathBuf;
use std::time::Duration;

/// One tick's result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticked {
    /// This worker's answers still pending (a failed tick counts one).
    pub pending: usize,
    /// Something else keeps it alive: an enabled remote machine, or agents
    /// whose blocked state it watches.
    pub keep_alive: bool,
}

/// What the worker's life depends on besides its lock and records.
pub trait World {
    /// Called once the lock is held, before the first tick: the worker
    /// migrates first (every other process opens without migrating) and
    /// decides whether it is herdr's own install.
    fn begin(&mut self);
    /// herdr says this worker's server is not running.
    fn server_gone(&mut self) -> bool;
    /// The version installed now (herdr's current install), if known.
    fn installed_version(&mut self) -> Option<String>;
    /// Where the inbox is installed now: an update's successor's root.
    fn current_root(&mut self) -> Option<PathBuf>;
    /// The install herdr lists: a revision-wanted successor's root.
    fn listed_root(&mut self) -> Option<PathBuf>;
    /// The revision a newer release wants (the revision-wanted marker).
    fn wanted(&mut self) -> Option<i64>;
    /// The revision the database is stamped with.
    fn db_stamp(&mut self) -> Option<i64>;
    /// One round of the worker's duties.
    fn tick(&mut self) -> Ticked;
    /// This worker's answers pending now (asked once the lock is let go).
    fn pending_now(&mut self) -> usize;
    /// Starts the new install's worker (the lock is released); true if one
    /// started.
    fn start_successor(&mut self, root: Option<PathBuf>) -> bool;
    /// Starts the listed install's worker, which stands by if it is newer.
    fn spawn_newer(&mut self, root: Option<PathBuf>);
    /// Starts this code's worker unless one runs.
    fn ensure(&mut self) -> bool;
    /// Monotonic seconds.
    fn monotonic(&self) -> f64;
    /// Waits `d`.
    fn sleep(&mut self, d: Duration);
}

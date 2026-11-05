//! A scripted [`World`] on a clock that moves only when slept.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use herdr_agent_kit::plugin_env::HeldLock;

use super::super::{Ticked, World};
use crate::worker::lock::is_running;

#[derive(Default)]
pub struct Fake {
    pub t: f64,
    /// server_gone readings, then `false` (or `true` once stopped).
    pub gone: VecDeque<bool>,
    pub gone_checks: usize,
    /// After this many ticks the server goes away for good (ends a test
    /// that would otherwise serve forever).
    pub stop_after: Option<usize>,
    pub installed: Option<String>,
    pub current: Option<PathBuf>,
    pub listed: Option<PathBuf>,
    pub wanted: Option<i64>,
    pub stamp: Option<i64>,
    pub ticked: Ticked,
    pub ticks: usize,
    /// The first this many ticks panic.
    pub panics: usize,
    pub pending_now: usize,
    pub successor_starts: bool,
    pub calls: Vec<String>,
    /// The lock, to check what is held when.
    pub lock: PathBuf,
    /// Another worker's lock, let go after this many sleeps.
    pub other: Option<(HeldLock, usize)>,
    pub sleeps: usize,
}

impl Fake {
    pub fn new(lock: PathBuf) -> Self {
        Self {
            lock,
            ticked: Ticked {
                pending: 1,
                keep_alive: false,
            },
            ..Self::default()
        }
    }

    fn stopped(&self) -> bool {
        self.stop_after.is_some_and(|n| self.ticks >= n)
    }
}

impl World for Fake {
    fn begin(&mut self) {
        let holder = std::fs::read_to_string(crate::worker::handover::holder_path(&self.lock));
        self.calls
            .push(format!("begin {}", holder.unwrap_or_default().trim()));
    }
    fn server_gone(&mut self) -> bool {
        if self.stopped() {
            return true;
        }
        self.gone_checks += 1;
        self.gone.pop_front().unwrap_or(false)
    }
    fn installed_version(&mut self) -> Option<String> {
        self.installed.clone()
    }
    fn current_root(&mut self) -> Option<PathBuf> {
        self.current.clone()
    }
    fn listed_root(&mut self) -> Option<PathBuf> {
        self.listed.clone()
    }
    fn wanted(&mut self) -> Option<i64> {
        self.wanted
    }
    fn db_stamp(&mut self) -> Option<i64> {
        self.stamp
    }
    fn tick(&mut self) -> Ticked {
        self.ticks += 1;
        assert!(self.ticks > self.panics, "Question Text 345");
        self.ticked
    }
    fn pending_now(&mut self) -> usize {
        let held = is_running(&self.lock);
        self.calls.push(format!("pending_now held={held}"));
        self.pending_now
    }
    fn start_successor(&mut self, root: Option<PathBuf>) -> bool {
        let held = is_running(&self.lock);
        self.calls
            .push(format!("start_successor {root:?} held={held}"));
        self.successor_starts
    }
    fn spawn_newer(&mut self, root: Option<PathBuf>) {
        self.calls
            .push(format!("spawn_newer {root:?} at {:.0}", self.t));
    }
    fn ensure(&mut self) -> bool {
        self.calls.push("ensure".into());
        true
    }
    fn monotonic(&self) -> f64 {
        self.t
    }
    fn sleep(&mut self, d: Duration) {
        self.t += d.as_secs_f64();
        self.sleeps += 1;
        if self.other.as_ref().is_some_and(|(_, n)| self.sleeps >= *n) {
            self.other = None;
        }
    }
}

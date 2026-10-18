//! Real time that a test can end: the real-watcher tests stop in seconds even when the code
//! under test is broken (a watcher left running would hold the test for its 12 h max age).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use herdr_inbox::rewake::Clock;

use crate::common::A;
use crate::world::World;

/// The time from now while the test runs; once `ended`, far past every deadline of the
/// watcher (its max age, the lock wait), so any watcher ends at its next check, whatever its
/// owner check does. Sleeps wake within 50 ms of the end.
pub struct EndableClock<'a> {
    start: Instant,
    ended: &'a AtomicBool,
}

impl<'a> EndableClock<'a> {
    pub fn new(ended: &'a AtomicBool) -> EndableClock<'a> {
        EndableClock {
            start: Instant::now(),
            ended,
        }
    }
}

impl Clock for EndableClock<'_> {
    fn now(&self) -> f64 {
        if self.ended.load(Ordering::SeqCst) {
            1e12
        } else {
            self.start.elapsed().as_secs_f64()
        }
    }

    fn sleep(&self, seconds: f64) {
        let until = Instant::now() + Duration::from_secs_f64(seconds.max(0.0));
        while Instant::now() < until && !self.ended.load(Ordering::SeqCst) {
            std::thread::sleep(
                Duration::from_millis(50).min(until.saturating_duration_since(Instant::now())),
            );
        }
    }
}

/// Ends every real watcher of A when dropped, also while a failed assertion unwinds (a scope
/// joins its threads afterwards): the clocks jump past every deadline, and the owner file is
/// rewritten as well.
pub struct EndWatchers<'a>(pub &'a World, pub &'a AtomicBool);

impl Drop for EndWatchers<'_> {
    fn drop(&mut self) {
        self.1.store(true, Ordering::SeqCst);
        let dir = self.0.dir.path().join("rewake");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("{A}.owner")), "ended");
    }
}

/// Joins a watcher thread, but ends every watcher (and fails the test) when it has not ended
/// within `limit`: a broken owner check fails in seconds instead of hanging the run.
pub fn join_within<T>(
    handle: std::thread::ScopedJoinHandle<'_, T>,
    limit: Duration,
    ended: &AtomicBool,
) -> T {
    let deadline = Instant::now() + limit;
    while !handle.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let late = !handle.is_finished();
    if late {
        ended.store(true, Ordering::SeqCst);
    }
    let out = handle.join().unwrap();
    assert!(!late, "the watcher did not end within {limit:?}");
    out
}

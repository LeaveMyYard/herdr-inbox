//! The watcher's time: a monotonic clock and its sleep, replaceable in tests
//! (Python `rewake.run(clock=..., sleep=...)`).

use std::time::{Duration, Instant};

/// Seconds on a monotonic clock, and sleeping.
pub trait Clock {
    /// Now, in seconds.
    fn now(&self) -> f64;
    /// Sleep `seconds`.
    fn sleep(&self, seconds: f64);
}

/// The real clock.
#[derive(Debug)]
pub struct SystemClock(Instant);

impl SystemClock {
    /// A clock counting from now.
    pub fn new() -> SystemClock {
        SystemClock(Instant::now())
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        self.0.elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        std::thread::sleep(Duration::from_secs_f64(seconds.max(0.0)));
    }
}

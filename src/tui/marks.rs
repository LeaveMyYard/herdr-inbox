//! The panes' marks off the popup's thread (Python `service.enable_background`,
//! `service.later`, `service.drain`): a dismissal, an acknowledgement or a
//! send refreshes the asking pane's `$inbox_q` token and `❓N` tab prefix,
//! which waits for herdr and the tab-label lock; the popup only queues it.
//! One thread works the queue in order; on exit the popup waits a bounded
//! time for what it queued ([`Marks::drain`], Python's `drain(1.0)`).

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long the popup waits on exit for the marks it queued (Python
/// `on_unmount`'s `service.drain(1.0)`).
pub const DRAIN: Duration = Duration::from_secs(1);

/// The queue (cheap to clone: every clone feeds the one thread).
#[derive(Clone)]
pub struct Marks {
    tx: Sender<Option<String>>,
    counts: Arc<(Mutex<(u64, u64)>, Condvar)>,
}

impl Marks {
    /// Starts the thread; `update` refreshes one pane's marks (nothing for
    /// no pane). A panic in it is the end of that one job, not of the queue.
    pub fn start(mut update: impl FnMut(Option<&str>) + Send + 'static) -> Marks {
        let (tx, rx) = channel::<Option<String>>();
        let counts = Arc::new((Mutex::new((0, 0)), Condvar::new()));
        let done = Arc::clone(&counts);
        // a thread that cannot start leaves the queue closed: `push` then queues nothing
        let _ = thread::Builder::new()
            .name("inbox-background".into())
            .spawn(move || {
                for pane in rx {
                    let job = std::panic::AssertUnwindSafe(|| update(pane.as_deref()));
                    let _ = std::panic::catch_unwind(job);
                    let (lock, cv) = &*done;
                    if let Ok(mut c) = lock.lock() {
                        c.1 += 1;
                    }
                    cv.notify_all();
                }
            });
        Marks { tx, counts }
    }

    /// Queues `pane`'s refresh.
    pub fn push(&self, pane: Option<&str>) {
        let (lock, _) = &*self.counts;
        if let Ok(mut c) = lock.lock()
            && self.tx.send(pane.map(str::to_string)).is_ok()
        {
            c.0 += 1;
        }
    }

    /// Waits up to `timeout` for every queued refresh; `true` if all ran.
    pub fn drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (lock, cv) = &*self.counts;
        let Ok(mut c) = lock.lock() else { return false };
        while c.1 < c.0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match cv.wait_timeout(c, left) {
                Ok((guard, _)) => c = guard,
                Err(_) => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;

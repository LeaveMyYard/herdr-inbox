//! What a tool does to herdr after its store work (Python `service.later`
//! and `update_markers`): a notification, the pane's markers. Run off the
//! call, one at a time on a background thread, so herdr never holds up a
//! tool; what is queued when the server exits still runs, for at most
//! [`DRAIN`] (Python `atexit.register(service.drain, 1.0)`). A failure is
//! warned about on stderr and never stops the queue (a panic included) nor
//! reaches the agent: these are cosmetic.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::store::alerts::Effect;

/// How long queued effects may still run at exit.
pub const DRAIN: Duration = Duration::from_secs(1);

/// Carries out one effect.
pub trait Herald: Send + Sync + 'static {
    /// `herdr.notify(title, body)`.
    fn notify(&self, title: &str, body: &str);
    /// `service.update_markers(pane)`.
    fn markers(&self, pane: &str);
}

/// The background queue.
pub struct Effects {
    tx: Mutex<Option<Sender<Effect>>>,
    done: Mutex<Option<Receiver<()>>>,
}

impl Effects {
    /// A queue whose thread hands each effect to `herald`.
    pub fn start(herald: Arc<dyn Herald>) -> Effects {
        let (tx, rx) = channel::<Effect>();
        let (done_tx, done_rx) = channel();
        thread::spawn(move || {
            for e in rx {
                // one bad job never stops the queue (Python's loop catches every exception)
                let run = AssertUnwindSafe(|| {
                    let _job = crate::trace::step("background.job");
                    match e {
                        Effect::Notify { title, body } => herald.notify(&title, &body),
                        Effect::Markers(Some(pane)) => herald.markers(&pane),
                        Effect::Markers(None) => {}
                    }
                });
                if catch_unwind(run).is_err() {
                    let _ = writeln!(std::io::stderr(), "herdr-inbox: a background effect failed");
                }
            }
            let _ = done_tx.send(());
        });
        Effects {
            tx: Mutex::new(Some(tx)),
            done: Mutex::new(Some(done_rx)),
        }
    }

    /// Queues `effects`, in order.
    pub fn later(&self, effects: Vec<Effect>) {
        if let Ok(tx) = self.tx.lock()
            && let Some(tx) = tx.as_ref()
        {
            for e in effects {
                let _ = tx.send(e);
            }
        }
    }

    /// Closes the queue and waits up to `timeout` for what it holds; `true`
    /// if it all ran.
    pub fn drain(&self, timeout: Duration) -> bool {
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
        let deadline = Instant::now() + timeout;
        match self.done.lock().ok().and_then(|mut d| d.take()) {
            Some(rx) => rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_ok(),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests;

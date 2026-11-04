//! Background sends (Python `_send`): the popup never waits for the
//! database's write lock, herdr or delivery. Each [`Effect::Send`] runs on
//! its own thread with its own connection: it is recorded ([`record()`]),
//! handed to delivery ([`handoff`]), and reported back to the popup's
//! thread ([`Back::Sent`], or a toast when nothing was recorded).
//!
//! Closing the popup waits for every send in flight ([`Sender::wait`],
//! Python `action_quit`'s `workers.wait_for_complete`): an answer the
//! operator gave is never lost to the popup's exit. A report that comes after
//! the popup stopped taking events changes nothing.
//!
//! [`Effect::Send`]: inbox_core::popup::Effect::Send

pub mod handoff;
pub mod record;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender as Tx;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use inbox_core::popup::{Send, Severity};

use crate::store::Store;
use crate::store::busy::is_busy;
use crate::tui::back::Back;
use crate::tui::marks::Marks;
use crate::tui::toasts::Toast;
use handoff::{Handoff, hand_off};
use record::{Refused, record};

/// What a busy database says (Python `tui.BUSY`).
pub const BUSY: &str = "The inbox database is busy; try again in a moment";

/// Where sends run and report.
pub struct Sender {
    db_path: PathBuf,
    timeout: Duration,
    focus_pane: Option<String>,
    handoff: Arc<dyn Handoff>,
    marks: Marks,
    back: Tx<Back>,
    threads: Vec<JoinHandle<()>>,
}

impl Sender {
    /// Sends open their own connection to `db_path` (SQLite's busy timeout
    /// `timeout`); answers to `focus_pane` are typed while the popup is up.
    pub fn new(
        db_path: PathBuf,
        timeout: Duration,
        focus_pane: Option<String>,
        handoff: Arc<dyn Handoff>,
        marks: Marks,
        back: Tx<Back>,
    ) -> Sender {
        Sender {
            db_path,
            timeout,
            focus_pane,
            handoff,
            marks,
            back,
            threads: Vec::new(),
        }
    }

    /// Starts `send` in the background.
    pub fn send(&mut self, send: Send) {
        let job = Job {
            db_path: self.db_path.clone(),
            timeout: self.timeout,
            focus_pane: self.focus_pane.clone(),
            handoff: Arc::clone(&self.handoff),
            marks: self.marks.clone(),
            back: self.back.clone(),
        };
        match thread::Builder::new()
            .name("inbox-send".into())
            .spawn(move || job.run(&send))
        {
            Ok(handle) => self.threads.push(handle),
            Err(err) => {
                let text = format!("Not sent: could not start the send ({err})");
                let _ = self
                    .back
                    .send(Back::Toast(Toast::new(text, Severity::Warning)));
            }
        }
    }

    /// Some send is still running.
    pub fn in_flight(&mut self) -> bool {
        self.threads.retain(|t| !t.is_finished());
        !self.threads.is_empty()
    }

    /// Waits for every send in flight.
    pub fn wait(&mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// One send's thread.
struct Job {
    db_path: PathBuf,
    timeout: Duration,
    focus_pane: Option<String>,
    handoff: Arc<dyn Handoff>,
    marks: Marks,
    back: Tx<Back>,
}

impl Job {
    fn run(self, send: &Send) {
        let report = match self.work(send) {
            Ok(back) => back,
            Err(Refused::AlreadySending(qid)) => Back::Toast(Toast::new(
                format!("Q{qid} is already being delivered"),
                Severity::Warning,
            )),
            Err(Refused::Db(err)) => Back::Toast(Toast::new(failure(&err), Severity::Warning)),
        };
        let _ = self.back.send(report); // the popup may be gone: the send itself is done
    }

    fn work(&self, send: &Send) -> Result<Back, Refused> {
        let store = Store::open(&self.db_path, self.timeout).map_err(|e| match e {
            crate::store::OpenError::Sqlite(err) => Refused::Db(err),
            not_ready => Refused::Db(rusqlite::Error::ToSqlConversionFailure(
                not_ready.to_string().into(),
            )),
        })?;
        let marks = self.marks.clone();
        let rec = record(&store, send, &mut |pane| marks.push(pane))?;
        hand_off(
            &store,
            &rec.done,
            self.focus_pane.as_deref(),
            rec.force,
            &*self.handoff,
        )?;
        Ok(Back::Sent {
            done: rec.done.iter().map(|q| q.id).collect(),
            wanted: rec.wanted,
        })
    }
}

/// What a failed database step says: Python's notice for a busy database;
/// any other failure by its error (Python's popup would stop with a
/// traceback there, or call it busy).
pub fn failure(err: &rusqlite::Error) -> String {
    if is_busy(err) {
        BUSY.to_string()
    } else {
        format!("The inbox database failed: {err}")
    }
}

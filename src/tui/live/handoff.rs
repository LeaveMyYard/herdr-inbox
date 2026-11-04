//! The live [`Handoff`]: delivery in the send's own thread, for now.
//!
//! Python hands the answers for other panes to a detached `inbox deliver`
//! and the transcript check of the ones it typed to a detached `inbox
//! verify`, so the popup can close at once. Neither command is in this
//! binary yet (`deliver` is herdr-inbox #112; `verify` comes with the
//! worker's verify-later jobs, P5-E2), so until they are this delivers
//! every answer in the send's thread and verifies it there, as Python does
//! with `DETACHED_SEND` off: the same pass, the same claims, only the popup
//! closes after the check instead of before it. The worker start is the CLI's
//! seam (a no-op until P5-E2 spawns `<exe> worker`).

use std::path::PathBuf;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use inbox_core::deliver::Mode;

use crate::claude::Transcripts;
use crate::delivery::run::{
    Ctx, UNREADABLE_NOTIFY_AFTER, Unreadable, VERIFY_SECONDS, deliver_batch,
};
use crate::delivery::send::local_answers;
use crate::store::{Question, Store};
use crate::surface::Surface;
use crate::surface::live::Live;
use crate::tui::send::handoff::Handoff;

/// Delivery in this process.
pub struct InProcess {
    /// herdr.
    pub herdr: Herdr,
    /// The database (a send's own connection is opened per call).
    pub db_path: PathBuf,
    /// The inbox's state directory.
    pub state_dir: PathBuf,
    /// This herdr server's id.
    pub server: String,
    /// How long a statement waits for the write lock.
    pub timeout: std::time::Duration,
}

impl InProcess {
    fn pass(&self, store: &Store, qs: &[Question], force: bool) -> rusqlite::Result<()> {
        let local = local_answers(store, qs, false, &self.server, &mut |s| self.ensure(s))?;
        let mut surface = Live::new(self.herdr.clone(), warn);
        let mut notify = |title: &str, body: &str| surface.notify(title, body);
        let mut unreadable = Unreadable::after(UNREADABLE_NOTIFY_AFTER);
        let files = Transcripts::from_env();
        let mut ctx = Ctx {
            store,
            herdr: &self.herdr,
            files: &files,
            state_dir: &self.state_dir,
            notify: &mut notify,
            unreadable: &mut unreadable,
            verify_wait: VERIFY_SECONDS,
        };
        let mode = Mode {
            force,
            ..Mode::default()
        };
        deliver_batch(&mut ctx, &local, mode, None)
    }

    fn ensure(&self, _server: Option<&str>) {
        // P5-E2 (`<exe> worker`): wired with the CLI's `ensure_worker`
    }
}

impl Handoff for InProcess {
    fn ensure_worker(&self) {
        self.ensure(None);
    }

    fn deliver_here(&self, store: &Store, qs: &[Question], force: bool) -> rusqlite::Result<()> {
        self.pass(store, qs, force)
    }

    fn spawn_delivery(&self, qids: &[i64], force: bool) {
        let delivered = Store::open(&self.db_path, self.timeout)
            .map_err(|e| e.to_string())
            .and_then(|store| {
                let mut qs = Vec::new();
                for &q in qids {
                    qs.extend(store.get(q).map_err(|e| e.to_string())?);
                }
                self.pass(&store, &qs, force).map_err(|e| e.to_string())
            });
        if let Err(err) = delivered {
            warn(&format!(
                "could not deliver now; the worker will retry: {err}"
            ));
        }
    }
}

fn warn(msg: &str) {
    crate::tui::live::log(msg);
}

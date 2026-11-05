//! The live [`Handoff`] (Python `_send`'s detached branch): answers for
//! other panes go to a detached `herdr-inbox deliver` (Python
//! `spawn_delivery`); answers for the operator's own pane are typed by the
//! send's thread while the popup still holds the keyboard, and their
//! transcript check is handed off (Python `verify_later=spawn_verify`).
//!
//! This binary has no `inbox verify` yet, so the
//! hand-off is Python's own fallback for a check that cannot start: the typed
//! rows keep their claim, the claim goes stale after
//! [`CLAIM_STALE`](crate::store::claims::CLAIM_STALE), and the worker takes
//! them over, looking at the transcript before it types anything again. The
//! popup closes as soon as the answer is typed. TODO(before P6-E): spawn
//! `<exe> verify <job>` here.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::plugin_env::PluginEnv;
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
use crate::worker::process::ensure_worker_in;
use crate::worker::spawn::{reap_later, spawn_detached};
use crate::worker::spawn_env;

/// The live hand-off.
pub struct LiveHandoff {
    /// herdr.
    pub herdr: Herdr,
    /// The inbox's state directory (`worker.log` is there).
    pub state_dir: PathBuf,
    /// This herdr server's id.
    pub server: String,
    /// This binary.
    pub exe: Option<PathBuf>,
    /// The plugin's environment, which says where a worker's lock and database are.
    pub env: PluginEnv,
}

/// The detached sender's arguments (Python `spawn_delivery`'s
/// `deliver <ids> [--force]`).
pub fn deliver_args(qids: &[i64], force: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["deliver".into()];
    args.extend(qids.iter().map(|q| OsString::from(q.to_string())));
    if force {
        args.push("--force".into());
    }
    args
}

impl LiveHandoff {
    fn ensure(&self, server: Option<&str>) {
        match &self.exe {
            Some(exe) => {
                ensure_worker_in(&self.env, server, exe);
            }
            None => super::log("could not start the worker: this binary's path is unknown"),
        }
    }
}

/// Makes sure a worker runs, off the popup's thread (Python's mount:
/// `run_worker(thread=True)`): it retries what the popup answers.
pub fn ensure_in_background(handoff: &Arc<LiveHandoff>) {
    let handoff = Arc::clone(handoff);
    std::thread::spawn(move || handoff.ensure_worker());
}

impl Handoff for LiveHandoff {
    fn ensure_worker(&self) {
        self.ensure(None);
    }

    fn deliver_here(&self, store: &Store, qs: &[Question], force: bool) -> rusqlite::Result<()> {
        let local = local_answers(store, qs, false, &self.server, &mut |s| self.ensure(s))?;
        let mut surface = Live::new(self.herdr.clone(), super::log);
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
        // TODO(before P6-E): a detached `verify` takes the job; until then the worker's
        // stale-claim takeover checks the transcript (Python's fallback). Live code
        // behind herdr: no test covers passing `Some` here.
        let mut later = |_job: serde_json::Value| {
            super::log("typed into the operator's pane; the worker checks the transcript");
        };
        deliver_batch(&mut ctx, &local, mode, Some(&mut later))
    }

    fn spawn_delivery(&self, qids: &[i64], force: bool) {
        let log = self.state_dir.join("worker.log");
        let started = self.exe.as_ref().map_or_else(
            || Err(std::io::Error::other("this binary's path is unknown")),
            |exe| {
                let stderr = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                    .ok();
                let env = spawn_env(std::env::vars_os(), "".as_ref());
                spawn_detached(
                    exe.as_os_str(),
                    &deliver_args(qids, force),
                    None,
                    &env,
                    stderr,
                )
            },
        );
        match started {
            Ok(child) => reap_later(child),
            // recorded, not yet claimed: the worker `ensure` started retries them
            Err(err) => super::log(&format!(
                "could not start the delivery; the worker will retry: {err}"
            )),
        }
    }
}

#[cfg(test)]
mod tests;

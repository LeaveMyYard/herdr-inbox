//! [`Live`]: the run loop's real [`World`]. herdr answers whether its server
//! is gone and where the inbox is installed, the disk holds the versions,
//! the marker and the stamp, and [`crate::worker::spawn`] starts workers.
//! What one tick does (delivery, verification, alerts, remotes) and the
//! migration come in as [`Duties`] ([`crate::worker::duties::live::LiveDuties`]),
//! so this lifecycle is complete and tested without them.

mod core;
mod db;
mod herdr;
mod install;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use herdr_agent_kit::herdr_api::blocking::Herdr;

pub use core::{Core, absent};
pub use db::db_stamp;
pub use herdr::{listed_root, server_gone};
pub use install::{Install, own_checkout, running_version, version_at};

use super::WorkerEnv;
use super::run::{Ticked, World};
use super::spawn;
use crate::store::revision_wanted;

/// What a worker does while it holds the lock.
pub trait Duties {
    /// Brings the database forward; runs once, before the first tick. A
    /// failure is the duties' own to report; the first tick tries again.
    fn migrate(&mut self);
    /// One round. An error counts as one answer pending (Python: a failed
    /// tick keeps the worker alive).
    fn tick(&mut self) -> Ticked;
    /// This worker's answers pending now.
    fn pending_now(&mut self) -> usize;
}

/// The real world of a worker for [`WorkerEnv::server`].
pub struct Live<D: Duties> {
    env: WorkerEnv,
    herdr: Herdr,
    install: Install,
    exe: PathBuf,
    duties: D,
    core: Option<Core>,
    origin: Instant,
}

impl<D: Duties> Live<D> {
    /// A worker of `env` talking to `herdr`, running from the checkout
    /// `install` (see [`own_checkout`]), started again as `exe worker` by
    /// [`World::ensure`], doing `duties`.
    pub fn new(env: WorkerEnv, herdr: Herdr, install: Install, exe: PathBuf, duties: D) -> Self {
        Self {
            env,
            herdr,
            install,
            exe,
            duties,
            core: None,
            origin: Instant::now(),
        }
    }

    /// The core worker: its server is `core`, not a herdr one.
    pub fn on_core(mut self, core: Core) -> Self {
        self.core = Some(core);
        self
    }

    /// Its duties (the tick's state).
    pub fn duties(&self) -> &D {
        &self.duties
    }
}

impl<D: Duties> World for Live<D> {
    fn begin(&mut self) {
        self.duties.migrate();
        let herdr = &self.herdr;
        self.install.follows(|| listed_root(herdr)); // decided now, on herdr's first answer
    }

    fn server_gone(&mut self) -> bool {
        match &self.core {
            Some(core) => core.gone(),
            None => server_gone(&self.herdr),
        }
    }

    fn installed_version(&mut self) -> Option<String> {
        self.current_root().and_then(|root| version_at(&root))
    }

    fn current_root(&mut self) -> Option<PathBuf> {
        let herdr = &self.herdr;
        self.install.current(|| listed_root(herdr))
    }

    fn listed_root(&mut self) -> Option<PathBuf> {
        listed_root(&self.herdr)
    }

    fn wanted(&mut self) -> Option<i64> {
        revision_wanted::wanted(&self.env.db, revision_wanted::unix_now())
    }

    fn db_stamp(&mut self) -> Option<i64> {
        db_stamp(&self.env.db)
    }

    fn tick(&mut self) -> Ticked {
        self.duties.tick()
    }

    fn pending_now(&mut self) -> usize {
        self.duties.pending_now()
    }

    fn start_successor(&mut self, root: Option<PathBuf>) -> bool {
        spawn::start_successor(&self.env, root.as_deref())
    }

    fn spawn_newer(&mut self, root: Option<PathBuf>) {
        spawn::spawn_newer(&self.env, root.as_deref());
    }

    fn ensure(&mut self) -> bool {
        spawn::ensure(&self.env, &self.exe)
    }

    fn monotonic(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[cfg(test)]
mod tests;

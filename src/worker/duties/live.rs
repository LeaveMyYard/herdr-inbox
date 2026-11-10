//! [`LiveDuties`]: the worker's [`Duties`] in the real world (Python
//! `worker._loop`'s tick half): the database opened migrating each tick
//! (and its unverified acknowledgements re-checked, as Python's `Store`
//! open does), the remote thread started once a saved machine is enabled,
//! the tick run, and what keeps the worker alive reported.

use std::io::Write;
use std::time::{Duration, Instant};

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::alerts::scan_blocked;
use super::retry::pending;
use super::tick::tick;
use super::unverified::reconcile_unverified;
use super::{Memory, Round, warn};
use crate::claude::Transcripts;
use crate::delivery::run::{Ctx, UNREADABLE_NOTIFY_AFTER, Unreadable, VERIFY_SECONDS};
use crate::delivery::terminal::Terminals;
use crate::store::Store;
use crate::store::alerts::BlockedScan;
use crate::store::hosts::CORE_SERVER;
use crate::surface::Surface;
use crate::surface::live::Live;
use crate::surface::markers::Markers;
use crate::tabs::lock::LOCK_TIMEOUT;
use crate::worker::WorkerEnv;
use crate::worker::live::Duties;
use crate::worker::remote::{Machines, RemoteEnv, sync_loop};
use crate::worker::run::Ticked;

/// The database's busy timeout (Python `Store(timeout=5)`).
pub const DB_TIMEOUT: Duration = Duration::from_secs(5);

/// A worker's duties for the server of its [`WorkerEnv`].
pub struct LiveDuties {
    env: WorkerEnv,
    herdr: Herdr,
    core: Box<dyn Terminals>,
    files: Transcripts,
    config: ConfigFile,
    remote: RemoteEnv,
    remote_started: bool,
    machines: Machines,
    unreadable: Unreadable,
    memory: Memory,
    scan: BlockedScan,
    /// Agents the last blocked scan saw (Python's module-level `watching`:
    /// a tick that fails before its scan keeps the previous count).
    watching: usize,
    origin: Instant,
}

fn say(msg: &str) {
    let _ = writeln!(std::io::stderr(), "inbox: {msg}"); // a full disk never ends the worker
}

impl LiveDuties {
    /// The duties of `env`'s worker, talking to `herdr` and `core`, reading
    /// `files`, with the plugin's `config` and the remote thread's `remote`.
    pub fn new(
        env: WorkerEnv,
        herdr: Herdr,
        core: Box<dyn Terminals>,
        files: Transcripts,
        config: ConfigFile,
        remote: RemoteEnv,
    ) -> Self {
        LiveDuties {
            env,
            herdr,
            core,
            files,
            config,
            remote,
            remote_started: false,
            machines: Machines::default(),
            unreadable: Unreadable::after(UNREADABLE_NOTIFY_AFTER),
            memory: Memory::default(),
            scan: BlockedScan::default(),
            watching: 0,
            origin: Instant::now(),
        }
    }

    /// The database as the worker opens it each tick: migrating, then its
    /// unverified acknowledgements re-checked (Python `Store(migrate=True)`).
    fn open(&self) -> Result<Store, String> {
        let store = Store::open_migrating(&self.env.db, DB_TIMEOUT).map_err(|e| e.to_string())?;
        crate::trace::timed("store.reconcile", || {
            reconcile_unverified(&store, &self.files)
        })
        .map_err(|e| e.to_string())?;
        Ok(store)
    }

    fn server(&self) -> String {
        self.env.server.to_string_lossy().into_owned()
    }

    /// This is the core worker: no herdr server, so no blocked scan, markers,
    /// notices or remote machines. Those are a herdr worker's.
    fn on_core(&self) -> bool {
        self.env.server == CORE_SERVER
    }

    fn run_tick(&mut self, store: &Store) -> rusqlite::Result<usize> {
        let server = self.server();
        let on_core = self.on_core();
        let state_dir = self.env.state_dir().to_path_buf();
        let mut surface = Live::new(self.herdr.clone(), say);
        let mut marks_surface = Live::new(self.herdr.clone(), say);
        let mut notify = |t: &str, b: &str| {
            if !on_core {
                surface.notify(t, b);
            }
        };
        let mut ctx = Ctx {
            store,
            herdr: &self.herdr,
            core: &*self.core,
            files: &self.files,
            state_dir: &state_dir,
            notify: &mut notify,
            unreadable: &mut self.unreadable,
            verify_wait: VERIFY_SECONDS,
        };
        let mut round = Round {
            ctx: &mut ctx,
            server: &server,
        };
        let (scan, config, db, watching) = (
            &mut self.scan,
            &mut self.config,
            &self.env.db,
            &mut self.watching,
        );
        let t = self.origin.elapsed().as_secs_f64();
        let mut blocked = |round: &mut Round<'_, '_>| {
            if on_core {
                return 0;
            }
            let mut warn_fn = |m: &str| say(m);
            let mut marks = |pane: Option<&str>| {
                Markers {
                    store,
                    db_path: db,
                    herdr: &mut marks_surface,
                    config: &mut *config,
                    lock_wait: LOCK_TIMEOUT,
                    warn: &mut warn_fn,
                }
                .update(pane);
            };
            *watching = scan_blocked(round, scan, t, &mut marks);
            *watching
        };
        Ok(tick(&mut round, &mut self.memory, &mut blocked)?.pending)
    }
}

impl Duties for LiveDuties {
    fn migrate(&mut self) {
        // migrate first: the remote thread and every other process open without migrating
        if let Err(e) = Store::open_migrating(&self.env.db, DB_TIMEOUT) {
            say(&format!("worker migration failed: {e}"));
        }
    }

    fn tick(&mut self) -> Ticked {
        let on_core = self.on_core();
        let remotes = !on_core && !self.machines.enabled(&self.herdr).is_empty();
        if remotes && !self.remote_started {
            let env = self.remote.clone();
            let started = std::thread::Builder::new()
                .name("inbox-remote".into())
                .spawn(move || sync_loop(env));
            self.remote_started = started.is_ok();
        }
        let pending = match self.open() {
            Ok(store) => self.run_tick(&store).unwrap_or_else(|e| {
                warn("worker tick failed", &e);
                1 // a failed tick keeps the worker alive
            }),
            Err(e) => {
                warn("worker tick failed", &e);
                1
            }
        };
        Ticked {
            pending,
            keep_alive: on_core || remotes || self.watching > 0, // the core worker lives with the core
        }
    }

    fn pending_now(&mut self) -> usize {
        let Ok(store) = self.open() else {
            return 0; // Python: an exception here starts nobody
        };
        let server = self.server();
        let state_dir = self.env.state_dir().to_path_buf();
        let mut notify = |_: &str, _: &str| {};
        let mut ctx = Ctx {
            store: &store,
            herdr: &self.herdr,
            core: &*self.core,
            files: &self.files,
            state_dir: &state_dir,
            notify: &mut notify,
            unreadable: &mut self.unreadable,
            verify_wait: VERIFY_SECONDS,
        };
        pending(&Round {
            ctx: &mut ctx,
            server: &server,
        })
        .unwrap_or(0)
    }
}

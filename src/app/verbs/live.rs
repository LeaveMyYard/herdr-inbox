//! The real world behind a verb: this process's environment, the database,
//! and herdr, asked only whether a server runs. Starting a sender is all it
//! does towards an agent.

use std::ffi::OsStr;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;
use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::route::{Effects, Mark};
use super::{Ctx, run};
use crate::store::Store;
use crate::surface::live::Live as Surface;
use crate::surface::markers::Markers;
use crate::tabs::settings::PLUGIN_ID;
use crate::worker::live::server_gone;
use crate::worker::process::{ensure_worker, own_exe};
use crate::worker::spawn::start_sender;

/// The whole command's budget, well inside `exec`'s 30 s.
pub const BUDGET: Duration = Duration::from_secs(5);
/// What the database may take of it: the rest is for the steps after the commit.
pub const DB_WAIT: Duration = Duration::from_millis(3500);
/// The longest one question to a server takes.
const PROBE: Duration = Duration::from_millis(1000);

/// `herdr-inbox app`: the exit code. Prints what the command says.
pub fn main(argv: &[String]) -> i32 {
    let env = PluginEnv::from_env();
    let db = inbox::db_path(&env);
    let mut world = World {
        deadline: Instant::now() + BUDGET,
        state_dir: inbox::state_dir(&env),
        exe: own_exe(),
        db: db.clone(),
        env,
    };
    let mut ctx = Ctx {
        db: &db,
        db_wait: DB_WAIT,
        core_delivery: CORE_DELIVERY,
        fx: &mut world,
    };
    let reply = run(argv, &mut ctx);
    let _ = std::io::stdout().write_all(reply.stdout.as_bytes());
    let _ = std::io::stderr().write_all(reply.stderr.as_bytes());
    reply.exit
}

/// Core delivery exists. Off until B3 lands: then every core answer here is
/// `held`, and no worker is asked for. To turn it on, set this and have
/// [`World::core_gone`] ask `hc`.
pub const CORE_DELIVERY: bool = false;

struct World {
    deadline: Instant,
    state_dir: PathBuf,
    exe: PathBuf,
    db: PathBuf,
    env: PluginEnv,
}

impl World {
    fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn herdr(&self, server: Option<&str>, timeout: Duration) -> Herdr {
        let herdr = match server {
            Some(socket) => Herdr::new(ServerId::new(socket)),
            None => Herdr::from_env(),
        };
        herdr.with_timeout(timeout)
    }
}

fn warn(_: &str) {} // marks are cosmetic: nothing to say to a mod

impl Effects for World {
    fn ensure_worker(&mut self, server: Option<&str>) {
        ensure_worker(server);
    }

    fn server_gone(&mut self, server: Option<&str>) -> bool {
        let left = self.left();
        !left.is_zero() && server_gone(&self.herdr(server, left.min(PROBE)))
    }

    fn core_gone(&mut self) -> bool {
        false // not reached while CORE_DELIVERY is off
    }

    fn start_sender(&mut self, questions: &[i64], force: bool, socket: Option<&str>) {
        // the answers are recorded and the worker is ensured: it retries them
        let _ = start_sender(
            Some(&self.exe),
            &self.state_dir,
            questions,
            force,
            socket.map(OsStr::new),
        );
    }

    fn refresh_marks(&mut self, marks: &[Mark]) {
        for mark in marks {
            let left = self.left();
            if left < Duration::from_millis(100) {
                return;
            }
            let Ok(store) = Store::open(&self.db, left / 4) else {
                return;
            };
            let herdr = self.herdr(mark.server.as_deref(), left / 10);
            let mut config = ConfigFile::resolve(&self.env, || {
                herdr.plugin_config_dir(PLUGIN_ID).ok().flatten()
            });
            let mut surface = Surface::new(herdr, warn);
            let mut say = |_: &str| {};
            Markers {
                store: &store,
                db_path: &self.db,
                herdr: &mut surface,
                config: &mut config,
                lock_wait: left / 8,
                warn: &mut say,
            }
            .update(Some(&mark.pane));
        }
    }
}

//! What a worker's life depends on outside this process ([`WorkerEnv`]),
//! its waits ([`Waits`]) and the environment it gives the workers it starts
//! ([`spawn_env`]).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use herdr_agent_kit::internal::plugin_paths::inbox::server_key;

use super::handover::STANDBY_MAX;

/// Seconds between ticks (Python `worker.TICK`).
pub const TICK: Duration = Duration::from_secs(5);
/// Idle this long (nothing pending, nothing kept alive): the worker exits
/// (Python `worker.IDLE_EXIT`).
pub const IDLE_EXIT: Duration = Duration::from_secs(600);
/// herdr said its server is gone on every check for this long: the worker
/// exits (Python `worker.SERVER_GONE_EXIT`).
pub const SERVER_GONE_EXIT: Duration = Duration::from_secs(10);
/// How long a handover waits for a Rust successor to hold the lock, or to
/// stand by (Python `worker.RUST_HANDOVER_WAIT`).
pub const RUST_HANDOVER_WAIT: Duration = Duration::from_secs(5);

/// The variable that sets [`Waits::tick`], in seconds (Python's).
pub const TICK_VAR: &str = "HERDR_INBOX_WORKER_TICK";
/// The variable that sets [`Waits::idle_exit`], in seconds (Python's).
pub const IDLE_EXIT_VAR: &str = "HERDR_INBOX_WORKER_IDLE_EXIT";
/// The variable that sets [`Waits::standby_max`], in seconds (Python
/// `worker_handover.STANDBY_MAX_ENV`).
pub const STANDBY_MAX_VAR: &str = "HERDR_INBOX_STANDBY_MAX";

/// How long a worker waits for what.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waits {
    /// Between two ticks.
    pub tick: Duration,
    /// Idle this long: exit.
    pub idle_exit: Duration,
    /// The longest a standby waits for the lock.
    pub standby_max: Duration,
    /// For a Rust successor to take the lock or stand by.
    pub handover_wait: Duration,
    /// herdr's "server not running" for this long: exit.
    pub server_gone_exit: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Self {
            tick: TICK,
            idle_exit: IDLE_EXIT,
            standby_max: STANDBY_MAX,
            handover_wait: RUST_HANDOVER_WAIT,
            server_gone_exit: SERVER_GONE_EXIT,
        }
    }
}

impl Waits {
    /// The defaults, with [`TICK_VAR`], [`IDLE_EXIT_VAR`] and
    /// [`STANDBY_MAX_VAR`] read through `var` as Python reads them (seconds,
    /// a float). A value that is not a finite, non-negative number keeps the
    /// default (where Python would fail to start).
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let secs = |name: &str, default: Duration| {
            var(name)
                .filter(|v| !v.is_empty())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .and_then(|s| Duration::try_from_secs_f64(s).ok())
                .unwrap_or(default)
        };
        let d = Self::default();
        Self {
            tick: secs(TICK_VAR, d.tick),
            idle_exit: secs(IDLE_EXIT_VAR, d.idle_exit),
            standby_max: secs(STANDBY_MAX_VAR, d.standby_max),
            ..d
        }
    }
}

/// Everything the worker's life depends on outside this process. Nothing in
/// [`crate::worker`] reads the process environment: only a binary's `main`
/// builds this, so a test never reaches the operator's database, lock or
/// herdr.
#[derive(Clone, Debug)]
pub struct WorkerEnv {
    /// The inbox database (its directory holds the lock and its records).
    pub db: PathBuf,
    /// The herdr server this worker serves (its socket path).
    pub server: OsString,
    /// The schema revision this code migrates to.
    pub revision: i64,
    /// The version of the code this process runs, compared with the
    /// version installed now: always [`running_version`] of its own
    /// checkout (read from disk at start, never the compiled version alone).
    ///
    /// [`running_version`]: super::live::running_version
    pub version: String,
    /// How long it waits for what.
    pub waits: Waits,
    /// The environment a spawned worker gets ([`spawn_env`]).
    pub spawn_env: Vec<(OsString, OsString)>,
    /// `ensure` starts no worker (Python `HERDR_INBOX_NO_WORKER`).
    pub no_worker: bool,
}

impl WorkerEnv {
    /// The database's directory.
    pub fn state_dir(&self) -> &Path {
        self.db.parent().unwrap_or(Path::new("."))
    }

    /// `worker-<sha1(server)[:12]>.lock` beside the database: one worker per
    /// herdr server (Python `worker._lock_path`).
    pub fn lock_path(&self) -> PathBuf {
        self.state_dir()
            .join(format!("worker-{}.lock", server_key(&self.server)))
    }
}

/// The environment a worker starts others with (Python `_spawn_env`): `vars`
/// without its venv selection (every `UV_*` and `VIRTUAL_ENV`: an MCP
/// server's `UV_PROJECT_ENVIRONMENT` must never make uv resync that venv for
/// the herdr checkout), with `HERDR_SOCKET_PATH` set to `server` (an empty
/// `server` keeps the inherited one, as Python's `server=None`).
pub fn spawn_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    server: &OsStr,
) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = vars
        .into_iter()
        .filter(|(k, _)| {
            let k = k.as_encoded_bytes();
            let socket = k == b"HERDR_SOCKET_PATH" && !server.is_empty();
            !k.starts_with(b"UV_") && k != b"VIRTUAL_ENV" && !socket
        })
        .collect();
    if !server.is_empty() {
        out.push(("HERDR_SOCKET_PATH".into(), server.to_owned()));
    }
    out
}

#[cfg(test)]
mod tests;

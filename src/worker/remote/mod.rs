//! The worker's remote thread (Python `remote.loop`, `sync_all_parallel`,
//! `enabled_machines`, `settings`; docs/REMOTE.md): every
//! `remote_sync_seconds` the outbox expiry sweep, then each enabled saved
//! machine pulled into the mirror and, if the pull worked, its outbox
//! flushed. One thread per machine (at most 8), so a hung machine never
//! delays the others. The pulls, flushes and sweep are the store's
//! ([`crate::store::mirror`]); this is their scheduling and their ssh.

pub mod ssh;

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{Machine, Sanitized, clean};
use herdr_agent_kit::plugin_env::config::ConfigFile;
use serde_json::{Map, Value};

use crate::store::Store;
use crate::store::mirror::Notice;
use ssh::SshEnv;

/// How long the machine list is reused (Python `enabled_machines(ttl=60)`).
pub const MACHINES_TTL: Duration = Duration::from_secs(60);
/// Machines synced at once at most (Python's `ThreadPoolExecutor(max_workers=min(8, n))`).
pub const MAX_PARALLEL: usize = 8;
/// The default `remote_sync_seconds`.
pub const SYNC_SECONDS: f64 = 5.0;

/// What the remote thread works with.
#[derive(Clone)]
pub struct RemoteEnv {
    /// The inbox database.
    pub db: PathBuf,
    /// herdr (for `machine list` and the notifications).
    pub herdr: Herdr,
    /// How ssh is run.
    pub ssh: SshEnv,
    /// The plugin's `config.toml` directory, if known.
    pub config_dir: Option<PathBuf>,
}

/// `herdr machine list`'s enabled machines, reused for [`MACHINES_TTL`].
#[derive(Default)]
pub struct Machines {
    cached: Option<(Instant, Vec<Machine>)>,
}

impl Machines {
    /// The enabled machines (Python `enabled_machines`): herdr's answer, or
    /// none when it cannot be asked.
    pub fn enabled(&mut self, herdr: &Herdr) -> Vec<Machine> {
        if let Some((at, found)) = &self.cached
            && at.elapsed() < MACHINES_TTL
        {
            return found.clone();
        }
        let found: Vec<Machine> = herdr
            .machine_list()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.enabled)
            .collect();
        self.cached = Some((Instant::now(), found.clone()));
        found
    }
}

/// `remote_sync_seconds` from `config.toml` (Python `float(s.get(...) or 5)`).
/// A value Python's `float()` refuses, or one too large to sleep for (Python's
/// `time.sleep` raises and ends the thread), is the default here.
pub fn sync_seconds(table: &Map<String, Value>) -> f64 {
    let v = match table.get("remote_sync_seconds") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match v {
        Some(s) if s > 0.0 && Duration::try_from_secs_f64(s).is_ok() => s,
        _ => SYNC_SECONDS, // `or 5`: a zero is the default too
    }
}

fn notify(herdr: &Herdr, notices: Vec<Notice>) {
    for n in notices {
        let (title, body) = (clean(&n.title, 200), clean(&n.body, 300));
        let _ = herdr.notify(
            &Sanitized::new(&title),
            &Sanitized::new(&body),
            Some("request"),
        );
    }
}

fn warn(what: &str, err: &dyn std::fmt::Display) {
    eprintln!("inbox: remote {what}: {err}");
}

/// One machine (Python `_sync_one`): pulled on a connection of its own, and
/// flushed if the pull worked.
pub fn sync_one(env: &RemoteEnv, m: &Machine) {
    let Some(id) = m.id.as_deref() else {
        return warn(m.label.as_deref().unwrap_or("?"), &"no machine id");
    };
    let run = || -> Result<(), String> {
        let store = Store::open(&env.db, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let mut ssh = |t: &_, c: &_, i: Option<&str>| ssh::run(&env.ssh, t, c, i);
        let (ok, notices) = store
            .pull(
                id,
                m.label.as_deref(),
                m.target.as_deref(),
                SystemTime::now(),
                &mut ssh,
            )
            .map_err(|e| e.to_string())?;
        notify(&env.herdr, notices);
        if ok {
            let mut clock = SystemTime::now;
            let flushed = store
                .flush(id, &mut clock, &mut ssh)
                .map_err(|e| e.to_string())?;
            notify(&env.herdr, flushed.notices);
            if let Some(why) = flushed.aborted {
                warn(id, &why);
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        warn(m.label.as_deref().unwrap_or(id), &e); // one machine never stops the others
    }
}

/// One round (Python `sync_all_parallel`): the expiry sweep over every
/// machine, then the machines `enabled` lists (asked after the sweep, as
/// Python) at once.
pub fn sync_all(env: &RemoteEnv, enabled: impl FnOnce() -> Vec<Machine>) {
    let sweep = || -> Result<Vec<Notice>, String> {
        let store = Store::open(&env.db, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        store
            .expire_sweep(&mut SystemTime::now)
            .map_err(|e| e.to_string())
    };
    match sweep() {
        Ok(notices) => notify(&env.herdr, notices),
        Err(e) => warn("expiry sweep failed", &e),
    }
    each_machine(&enabled(), &|m| sync_one(env, m));
}

/// `work` on every machine, by a pool of at most [`MAX_PARALLEL`] threads each
/// taking the next one. Python catches everything per machine: a panic in
/// one never ends the round.
fn each_machine(found: &[Machine], work: &(dyn Fn(&Machine) + Sync)) {
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..found.len().min(MAX_PARALLEL) {
            s.spawn(|| {
                while let Some(m) = found.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if std::panic::catch_unwind(AssertUnwindSafe(|| work(m))).is_err() {
                        warn(m.id.as_deref().unwrap_or("?"), &"panicked");
                    }
                }
            });
        }
    });
}

/// [`sync_all`] for the loop: Python's `loop` wraps the round in `try`, so a
/// panic anywhere in it ends the round, never the thread.
fn guarded_round(env: &RemoteEnv, enabled: impl FnOnce() -> Vec<Machine>) {
    if std::panic::catch_unwind(AssertUnwindSafe(|| sync_all(env, enabled))).is_err() {
        warn("round", &"panicked");
    }
}

/// The remote thread's loop: a round, then `remote_sync_seconds` (read
/// again each time, so an edit applies at once). Never returns.
pub fn sync_loop(env: RemoteEnv) {
    let mut machines = Machines::default();
    loop {
        guarded_round(&env, || machines.enabled(&env.herdr));
        let mut config = env.config_dir.as_ref().map(ConfigFile::in_dir);
        let table: Map<String, Value> = config
            .as_mut()
            .and_then(|c| c.load_as().ok())
            .unwrap_or_default();
        std::thread::sleep(Duration::from_secs_f64(sync_seconds(&table)));
    }
}

#[cfg(test)]
mod tests;

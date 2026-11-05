//! Starting workers (Python `worker.ensure`, `_start_rust_worker`,
//! `start_successor`, `_spawn_newer`, `_rust_stands_by`). Every worker is
//! started detached in a process group of its own. The only process ever
//! signalled is a Rust successor this process just spawned and still holds,
//! which did not take the lock (or stand by) in time ([`watch`]); a running
//! worker of anyone else never is.

mod binary;
mod child;
mod rate;

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub use binary::{
    PYTHON, PYTHON_WORKER, RUST_WORKER_BINARIES, python_worker_args, rust_worker_args,
    rust_worker_binary, uv_worker_args,
};
pub use child::{POLL, reap_later, spawn_detached, watch};
pub use rate::{ENSURE_EVERY, may_spawn, stamp_path};

use super::WorkerEnv;
use super::handover::{holder_revision, live_standby};
use super::lock::is_running;
pub use crate::store::revision_wanted::unix_now;

/// Starts `binary worker` in `root` after this worker released the lock;
/// true once some worker holds the lock. One that exits without it (no
/// `worker` subcommand, a crash), or has not taken it within the handover
/// wait, is no successor: it is killed with its group (anything that took
/// the lock since dies with it) and the caller starts the Python worker.
pub fn start_rust_worker(env: &WorkerEnv, binary: &Path, root: &Path) -> bool {
    let lock = env.lock_path();
    let args = rust_worker_args();
    match spawn_detached(binary.as_os_str(), &args, Some(root), &env.spawn_env, None) {
        Ok(child) => watch(
            child,
            env.waits.handover_wait,
            |_| is_running(&lock),
            || is_running(&lock),
        ),
        Err(_) => false,
    }
}

/// After an update, the new code's worker from `root` (herdr's current
/// install): its Rust worker if one is installed and takes the lock, else
/// its Python worker through `uv` ([`uv_worker_args`]). False if neither
/// could start (no root, no `uv`): then this worker keeps serving.
pub fn start_successor(env: &WorkerEnv, root: Option<&Path>) -> bool {
    let Some(root) = root else {
        return false;
    };
    if let Some(binary) = rust_worker_binary(root)
        && start_rust_worker(env, &binary, root)
    {
        return true;
    }
    let args = uv_worker_args(root);
    spawn_detached(OsStr::new("uv"), &args, None, &env.spawn_env, None)
        .map(reap_later)
        .is_ok()
}

/// Starts `binary worker` in `root` while this worker keeps the lock; true
/// once it stands by (its pid in the standby record). One that exits first
/// (no `worker` subcommand, not newer) or does not stand by within the
/// handover wait is killed with its group.
pub fn rust_stands_by(env: &WorkerEnv, binary: &Path, root: &Path) -> bool {
    let lock = env.lock_path();
    let args = rust_worker_args();
    match spawn_detached(binary.as_os_str(), &args, Some(root), &env.spawn_env, None) {
        Ok(child) => watch(
            child,
            env.waits.handover_wait,
            |c| live_standby(&lock, unix_now()).is_some_and(|(_, pid)| pid == c.id()),
            || false,
        ),
        Err(_) => false,
    }
}

/// A revision-wanted marker asked for a newer worker: `root`'s (the install
/// herdr lists) Rust worker if it stands by, else its Python worker
/// ([`python_worker_args`], in `root`). Either stands by only if it is
/// newer, and this worker then hands over; otherwise it exits.
pub fn spawn_newer(env: &WorkerEnv, root: Option<&Path>) {
    let Some(root) = root else {
        return;
    };
    if let Some(binary) = rust_worker_binary(root)
        && rust_stands_by(env, &binary, root)
    {
        return;
    }
    let args = python_worker_args(&root.join("src"));
    if let Ok(child) = spawn_detached(OsStr::new(PYTHON), &args, Some(root), &env.spawn_env, None) {
        reap_later(child);
    }
}

/// Starts this code's worker (`exe worker`, stderr appended to
/// `worker.log`) unless one runs, at most once per [`ENSURE_EVERY`]. A
/// worker of an older revision holding the lock, with no standby yet, gets
/// this revision's as a standby. True if one was started.
pub fn ensure(env: &WorkerEnv, exe: &Path) -> bool {
    if env.no_worker {
        return false;
    }
    let lock = env.lock_path();
    if is_running(&lock)
        && (holder_revision(&lock) >= env.revision || live_standby(&lock, unix_now()).is_some())
    {
        return false;
    }
    if !may_spawn(&lock, env.revision) {
        return false;
    }
    let _ = fs::create_dir_all(env.state_dir());
    let log = env.state_dir().join("worker.log");
    let err = OpenOptions::new().create(true).append(true).open(log).ok();
    let args = rust_worker_args();
    match spawn_detached(exe.as_os_str(), &args, None, &env.spawn_env, err) {
        Ok(child) => {
            reap_later(child);
            true
        }
        Err(e) => {
            // Python `log.warning("delivery worker: %s", exc)`
            let _ = writeln!(std::io::stderr(), "inbox: delivery worker: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests;

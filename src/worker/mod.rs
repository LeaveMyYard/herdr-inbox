//! The delivery worker's life (Python `worker.py`'s `run`, `_loop`,
//! `ensure`, `start_successor`, and `worker_handover.py`): one worker per
//! herdr server holds `worker-<key>.lock` ([`lock`]), hands it to a newer
//! revision ([`handover`]) or to an update's install ([`spawn`]), and exits
//! when idle or when its server is gone ([`run`]). [`live::Live`] is the
//! real world it runs in; what one tick does comes in as [`live::Duties`]
//! (slice P5-E2).
//!
//! The binary has no `worker` subcommand yet, on purpose: the Python
//! bridge's `start_successor` starts `target/release/herdr-inbox worker`
//! (or `bin/`) whenever it exists, so a `worker` without its duties would
//! take the lock and deliver nothing. It is wired with the duties.
//!
//! Nothing in this module reads the process environment: every path, the
//! server, the revision and the waits come in a [`WorkerEnv`] that only a
//! binary's `main` builds, so a test can never reach the operator's
//! database, lock or herdr. A process is signalled only when this process
//! spawned it in a group of its own and still holds its `Child` (a
//! successor that never took the lock); a running worker of anyone else is
//! never signalled: handover is cooperative, through the lock, the holder
//! and standby records and the revision-wanted marker.

pub mod duties;
mod env;
pub mod handover;
pub mod live;
pub mod lock;
pub mod remote;
pub mod run;
pub mod spawn;

pub use env::{
    IDLE_EXIT, IDLE_EXIT_VAR, RUST_HANDOVER_WAIT, SERVER_GONE_EXIT, STANDBY_MAX_VAR, TICK,
    TICK_VAR, Waits, WorkerEnv, spawn_env,
};

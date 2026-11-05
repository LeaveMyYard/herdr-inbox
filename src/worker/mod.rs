//! The delivery worker's life (Python `worker.py`'s `run`, `_loop`,
//! `ensure`, `start_successor`, and `worker_handover.py`): one worker per
//! herdr server holds `worker-<key>.lock` ([`lock`]), hands it to a newer
//! revision ([`handover`]) or to an update's install ([`spawn`]), and exits
//! when idle or when its server is gone ([`run`]). [`live::Live`] is the
//! real world it runs in; what one tick does comes in as [`live::Duties`].
//!
//! [`duties`] is what one tick does ([`duties::live::LiveDuties`] in the
//! real world) and [`remote`] the remote thread; [`process`] is the
//! binary's `worker [--ensure]` and every process's "make sure a worker
//! runs" (`ensure_worker`).
//!
//! Nothing in this module but [`process`] reads the process environment:
//! every path, the server, the revision and the waits come in a
//! [`WorkerEnv`] that only [`process`] builds (for the binary), so a test can never reach the operator's
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
pub mod process;
pub mod remote;
pub mod run;
pub mod spawn;

pub use env::{
    IDLE_EXIT, IDLE_EXIT_VAR, RUST_HANDOVER_WAIT, SERVER_GONE_EXIT, STANDBY_MAX_VAR, TICK,
    TICK_VAR, Waits, WorkerEnv, spawn_env,
};

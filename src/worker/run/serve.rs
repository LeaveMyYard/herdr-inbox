//! The loop a worker runs while it holds the lock (Python `worker._loop`):
//! tick until idle, until the herdr server is gone, until a newer worker
//! stands by, or until an update hands over to the new install's worker.

use std::panic::{AssertUnwindSafe, catch_unwind};

use herdr_agent_kit::plugin_env::HeldLock;

use super::{Ticked, World};
use crate::worker::WorkerEnv;
use crate::worker::duties::warn;
use crate::worker::handover::{SPAWN_EVERY, live_standby, write_holder};
use crate::worker::lock::hold;
use crate::worker::spawn::unix_now;

/// A newer worker stands by (Python `handed_over`): this one hands the lock
/// over; the standby takes it within a standby poll.
pub(super) fn handed_over(env: &WorkerEnv) -> bool {
    live_standby(&env.lock_path(), unix_now()).is_some_and(|(rev, _)| rev > env.revision)
}

/// A newer release asked (the revision-wanted marker) for a revision above
/// this worker's and the database's, and no worker stands by yet (Python
/// `_newer_wanted`).
pub(super) fn newer_wanted(env: &WorkerEnv, world: &mut impl World) -> bool {
    let Some(want) = world.wanted().filter(|w| *w > env.revision) else {
        return false;
    };
    if world.db_stamp().is_some_and(|s| s >= want) {
        return false;
    }
    live_standby(&env.lock_path(), unix_now()).is_none()
}

/// One tick. A panic ends the tick, never the worker, and counts as a failed
/// tick does, one answer pending. An unwinding tick drops its store, which
/// rolls back what it had open, and the next tick opens a new one.
///
/// This matches Python's `except Exception` around the tick only. Python also
/// catches around most steps, so an exception there skips that step and
/// delivery still runs. Here a panic ends the whole tick: one that repeats in
/// an early step stops delivery until the server goes, a handover or an update.
fn guarded_tick(world: &mut impl World) -> Ticked {
    catch_unwind(AssertUnwindSafe(|| world.tick())).unwrap_or_else(|_| {
        warn("worker tick failed", &"panicked");
        Ticked {
            pending: 1,
            keep_alive: false,
        }
    })
}

/// Serves while `held` is the lock; returns the exit code. `held` is `None`
/// on return when the lock was let go (a handover, or idle).
pub(super) fn serve(env: &WorkerEnv, world: &mut impl World, held: &mut Option<HeldLock>) -> i32 {
    let waits = env.waits;
    world.begin();
    let mut handover_failed = false;
    let mut gone_since = None::<f64>;
    let mut idle_since = world.monotonic();
    let mut spawned_at = world.monotonic() - SPAWN_EVERY.as_secs_f64();
    loop {
        // detached, so nothing else stops it: never outlive the server (plan §4 item 5)
        if world.server_gone() {
            let now = world.monotonic();
            match gone_since {
                None => gone_since = Some(now),
                Some(since) if now - since >= waits.server_gone_exit.as_secs_f64() => return 0,
                Some(_) => {}
            }
            world.sleep(waits.tick); // nothing can be delivered meanwhile: no tick
            continue;
        }
        gone_since = None;
        if handed_over(env) {
            return 0; // the standby takes the lock once run() lets it go
        }
        if newer_wanted(env, world) && world.monotonic() - spawned_at >= SPAWN_EVERY.as_secs_f64() {
            spawned_at = world.monotonic();
            let root = world.listed_root().or_else(|| world.current_root());
            world.spawn_newer(root); // if newer, it stands by and the next round hands over
        }
        let ticked = guarded_tick(world);
        let disk = world.installed_version();
        if disk.is_some_and(|d| d != env.version) && !handover_failed {
            *held = None; // updated: hand over
            let root = world.current_root();
            if world.start_successor(root) {
                return 0;
            }
            handover_failed = true; // keep serving with this code; try no more
            *held = hold(&env.lock_path()).ok().flatten();
            if held.is_none() {
                return 0; // someone else took over meanwhile
            }
            write_holder(&env.lock_path(), env.revision);
        }
        if ticked.pending > 0 || ticked.keep_alive {
            idle_since = world.monotonic();
        } else if world.monotonic() - idle_since >= waits.idle_exit.as_secs_f64() {
            *held = None;
            // an answer committed after this tick found the lock held and started nobody
            if world.pending_now() > 0 {
                world.ensure();
            }
            return 0;
        }
        world.sleep(waits.tick);
    }
}

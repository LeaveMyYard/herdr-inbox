//! The worker's run (Python `worker.run`): take the lock, or stand by for
//! an older holder, write the holder record, serve (its `serve` loop), then clear
//! the record if the lock is still this worker's.

mod serve;
mod world;

use std::path::PathBuf;
use std::time::Duration;

use herdr_agent_kit::plugin_env::HeldLock;

pub use world::{Ticked, World};

use super::WorkerEnv;
use super::handover::{Standby, clear_holder, holder_revision, wait_as_standby, write_holder};
use super::lock::{hold, hold_within};
use super::spawn::unix_now;

/// A standby's world: the lock tried without waiting, the world's clock.
struct AsStandby<'w, W: World> {
    lock: PathBuf,
    world: &'w mut W,
}

impl<W: World> Standby for AsStandby<'_, W> {
    type Held = HeldLock;
    fn take(&mut self) -> Option<HeldLock> {
        hold(&self.lock).ok().flatten()
    }
    fn monotonic(&self) -> f64 {
        self.world.monotonic()
    }
    fn unix(&self) -> f64 {
        unix_now()
    }
    fn sleep(&mut self, d: Duration) {
        self.world.sleep(d);
    }
}

/// Runs the worker; its exit code, always 0: it ended normally, another
/// worker runs for this server, or it handed over.
pub fn run(env: &WorkerEnv, world: &mut impl World) -> i32 {
    let lock = env.lock_path();
    let mut held = hold_within(&lock).ok().flatten();
    if held.is_none() && holder_revision(&lock) < env.revision {
        // an older worker holds it: stand by until it hands over (#50)
        let mut standby = AsStandby {
            lock: lock.clone(),
            world: &mut *world,
        };
        held = wait_as_standby(&lock, env.revision, env.waits.standby_max, &mut standby);
    }
    if held.is_none() {
        return 0;
    }
    write_holder(&lock, env.revision);
    let code = serve::serve(env, world, &mut held);
    if held.is_some() {
        clear_holder(&lock); // still ours while the lock is
    }
    drop(held);
    code
}

#[cfg(test)]
mod tests;

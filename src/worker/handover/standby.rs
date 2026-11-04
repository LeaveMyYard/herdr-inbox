//! A newer worker standing by for the lock an older one holds (Python
//! `worker_handover.wait_as_standby`).

use std::fs;
use std::path::Path;
use std::time::Duration;

use super::records::{holder_revision, live_standby, write};
use super::{STANDBY_HEARTBEAT, STANDBY_POLL, names_me, standby_path, start_time};

/// What a standby needs from its world: the lock, the clocks, sleeping.
pub trait Standby {
    /// The lock, once held.
    type Held;
    /// Takes the lock without waiting, if it is free.
    fn take(&mut self) -> Option<Self::Held>;
    /// Monotonic seconds.
    fn monotonic(&self) -> f64;
    /// Unix seconds (the heartbeat).
    fn unix(&self) -> f64;
    /// Waits `d`.
    fn sleep(&mut self, d: Duration);
}

/// Stands by for `lock` as a worker of `revision` for at most `max`: the
/// lock once taken, or `None` when `max` passed, when another live standby
/// of an equal or higher revision waits (one is enough), or when the holder
/// is of this revision or higher (a second newer worker won the lock). The
/// record goes when this returns, if it is still this process's.
pub fn wait_as_standby<S: Standby>(
    lock: &Path,
    revision: i64,
    max: Duration,
    world: &mut S,
) -> Option<S::Held> {
    let deadline = world.monotonic() + max.as_secs_f64();
    let me = std::process::id();
    let started = start_time(me); // once: on macOS it runs ps
    let path = standby_path(lock);
    let mut last_beat: Option<f64> = None;
    let out = loop {
        if world.monotonic() >= deadline {
            break None;
        }
        if let Some(held) = world.take() {
            break Some(held);
        }
        if live_standby(lock, world.unix()).is_some_and(|(rev, pid)| pid != me && rev >= revision) {
            break None;
        }
        let beat_due =
            last_beat.is_none_or(|b| world.monotonic() - b >= STANDBY_HEARTBEAT.as_secs_f64());
        if beat_due {
            // once a heartbeat: reading the holder's start time may run ps (macOS)
            if holder_revision(lock) >= revision {
                break None;
            }
            let record = format!("{revision} {me} {:.3} {started}\n", world.unix());
            write(&path, &record);
            last_beat = Some(world.monotonic());
        }
        world.sleep(STANDBY_POLL);
    };
    if names_me(&path) {
        let _ = fs::remove_file(&path);
    }
    out
}

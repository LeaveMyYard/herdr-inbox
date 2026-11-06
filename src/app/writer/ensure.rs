//! `app-feed --ensure`: makes sure one writer of this binary runs. Two mods
//! starting together wait on `ensure.lock`, so the second finds the writer
//! running. Only a proof counts as a start: `READY v1`, or a record that names
//! this live child with the lock held.

use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};

use super::control;
use super::files::{Paths, lock_within};
use super::ledger::{self, Ledger, MAX_FAILURES, now_ms};
use super::record::{Record, sha256};
use crate::worker::spawn_env;

/// What a child prints once it holds the lock and has written its record.
pub const READY: &[u8] = b"READY v1\n";
/// `--ensure` gives up on `ensure.lock` after this: a holder takes at most
/// [`ACK_WAIT`] plus [`STOP_WAIT`].
pub const ENSURE_WAIT: Duration = Duration::from_secs(7);
/// How long a child has to acknowledge.
pub const ACK_WAIT: Duration = Duration::from_secs(5);
/// How long a writer of another binary has to leave.
pub const STOP_WAIT: Duration = Duration::from_secs(2);
const ACK_POLL: Duration = Duration::from_millis(10);

/// What it took to find the writer running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    /// One of this binary ran already. Nothing started.
    Running,
    /// This call started one.
    Started,
}

/// Makes sure a writer of `exe` runs for `paths`. `Err`: none could be proven,
/// or starting is deferred or disabled.
pub fn ensure(paths: &Paths, exe: &Path) -> Result<Ensured, String> {
    paths.check_folder()?;
    control::enabled(paths)?; // no install, no lock file: nothing here to create
    let serial = paths.lock_file("ensure.lock").map_err(|e| e.to_string())?;
    if !lock_within(&serial, ENSURE_WAIT) {
        return Err("another --ensure did not finish in time".to_string());
    }
    control::enabled(paths)?; // an uninstall holds that lock while it disables
    let hash = sha256(exe).map_err(|e| format!("cannot hash this binary: {e}"))?;
    let writer = paths.lock_file("writer.lock").map_err(|e| e.to_string())?;
    if writer.try_lock().is_err() {
        if Record::read(paths).is_some_and(|r| r.binary_sha256 == hash) {
            ledger::clear(paths); // a proven writer ends any run of failures
            return Ok(Ensured::Running);
        }
        may_start(paths)?; // a writer that works stays until its successor may start
        stop_other(paths, &writer)?;
    }
    drop(writer); // the child takes it itself
    paths.remove("stop.request").map_err(|e| e.to_string())?;
    start(paths, exe)
}

/// Asks a writer of another binary to leave, and waits for it.
fn stop_other(paths: &Paths, writer: &File) -> Result<(), String> {
    File::create(paths.file("stop.request")).map_err(|e| e.to_string())?;
    let gone = lock_within(writer, STOP_WAIT);
    paths.remove("stop.request").map_err(|e| e.to_string())?;
    if gone {
        Ok(())
    } else {
        Err("a writer of another binary did not stop in time".to_string())
    }
}

/// The ledger, if it lets a spawn happen now.
fn may_start(paths: &Paths) -> Result<Ledger, String> {
    let ledger = Ledger::read(paths)?;
    if ledger.failures >= MAX_FAILURES {
        control::disable(paths)?;
        return Err("the writer failed to start too often; disabled".to_string());
    }
    if let Some(wait) = ledger.deferred(now_ms()) {
        return Err(format!("starting is deferred for {} ms", wait.as_millis()));
    }
    Ok(ledger)
}

fn start(paths: &Paths, exe: &Path) -> Result<Ensured, String> {
    let attempt = may_start(paths)?.count(paths, now_ms())?;
    let proven = spawn(paths, exe).and_then(|mut child| prove(paths, &mut child));
    match proven {
        Ok(()) => {
            ledger::clear(paths);
            Ok(Ensured::Started)
        }
        Err(why) if attempt.failures >= MAX_FAILURES => {
            control::disable(paths)?;
            Err(format!(
                "{why}; failed {MAX_FAILURES} times in a row, disabled"
            ))
        }
        Err(why) => Err(why),
    }
}

fn spawn(paths: &Paths, exe: &Path) -> Result<Child, String> {
    let env = spawn_env(std::env::vars_os(), &OsString::new());
    Command::new(exe)
        .args(["app-feed", "--writer", "--app-home"])
        .arg(&paths.home)
        .env_clear()
        .envs(env)
        .env("HERDR_APP_HOME", &paths.home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start the writer: {e}"))
}

/// Waits for the child's word. A child that stays silent may still be proven
/// by its record, but never by being alive.
fn prove(paths: &Paths, child: &mut Child) -> Result<(), String> {
    let mut pipe = child.stdout.take().ok_or("no pipe to the writer")?;
    let flags = fcntl_getfl(&pipe).map_err(|e| e.to_string())?;
    fcntl_setfl(&pipe, flags | OFlags::NONBLOCK).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + ACK_WAIT;
    let mut said = Vec::new();
    loop {
        let mut chunk = [0_u8; 64];
        match pipe.read(&mut chunk) {
            Ok(0) => return Err("the writer exited before it was ready".to_string()),
            Ok(n) => said.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.to_string()),
        }
        if said == READY {
            return Ok(());
        }
        if !READY.starts_with(&said) {
            return Err("the writer said something else".to_string());
        }
        if Instant::now() >= deadline {
            return registered(paths, child);
        }
        std::thread::sleep(ACK_POLL);
    }
}

fn registered(paths: &Paths, child: &mut Child) -> Result<(), String> {
    let alive = matches!(child.try_wait(), Ok(None));
    let named = Record::read(paths).is_some_and(|r| r.pid == child.id());
    let held = paths
        .lock_file("writer.lock")
        .is_ok_and(|lock| lock.try_lock().is_err());
    if alive && named && held {
        Ok(())
    } else {
        Err("the writer did not acknowledge in time".to_string())
    }
}

#[cfg(test)]
mod tests;

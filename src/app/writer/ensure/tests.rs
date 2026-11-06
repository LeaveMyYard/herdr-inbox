//! `--ensure` against stand-in writers: scripts that acknowledge, stay silent or die. A real
//! writer is tested in `tests/app/runtime.rs`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{ENSURE_WAIT, Ensured, ensure};
use crate::app::writer::control;
use crate::app::writer::files::Paths;
use crate::app::writer::ledger::{Ledger, MAX_FAILURES, now_ms};
use crate::app::writer::record::{Record, sha256};
use crate::test_exe;

struct Rig {
    dir: tempfile::TempDir,
    paths: Paths,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(&dir.path().join("home"));
    fs::create_dir_all(&paths.data).unwrap();
    fs::set_permissions(&paths.data, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(paths.file("control.json"), br#"{"v":1,"disabled":false}"#).unwrap();
    Rig { dir, paths }
}

impl Rig {
    /// A stand-in binary running `body`.
    fn exe(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        test_exe::install(&path, format!("#!/bin/sh\n{body}\n"));
        path
    }

    fn spawned(&self) -> usize {
        fs::read_to_string(self.dir.path().join("spawned")).map_or(0, |s| s.lines().count())
    }

    fn counts(&self) -> String {
        format!("echo x >> {}\n", self.dir.path().join("spawned").display())
    }

    fn failures(&self) -> u32 {
        Ledger::read(&self.paths).unwrap().failures
    }

    /// As if the spawn's deferral had passed.
    fn pass_the_wait(&self) {
        let l = Ledger::read(&self.paths).unwrap();
        let text = format!(r#"{{"v":1,"failures":{},"retry_at_ms":1}}"#, l.failures);
        fs::write(self.paths.file("start.json"), text).unwrap();
    }

    /// `path`'s stand-in holding the lock, as a live writer does, from this process.
    fn hold_writer(&self, exe: &Path) -> fs::File {
        let lock = self.paths.lock_file("writer.lock").unwrap();
        lock.try_lock().unwrap();
        Record::write(&self.paths, &sha256(exe).unwrap()).unwrap();
        lock
    }
}

#[test]
fn a_child_that_says_ready_is_proven_and_the_count_goes() {
    let r = rig();
    let exe = r.exe("ok", &format!("{}printf 'READY v1\\n'", r.counts()));
    assert_eq!(ensure(&r.paths, &exe), Ok(Ensured::Started));
    assert_eq!(r.spawned(), 1);
    assert_eq!(r.failures(), 0);
}

#[test]
fn a_child_that_dies_stays_counted_and_defers_the_next_spawn() {
    let r = rig();
    let exe = r.exe("dead", &format!("{}exit 3", r.counts()));
    assert!(ensure(&r.paths, &exe).is_err());
    assert_eq!(r.failures(), 1);
    assert!(ensure(&r.paths, &exe).is_err());
    assert_eq!(r.spawned(), 1, "deferred, not spawned");
    assert_eq!(r.failures(), 1, "and not counted");
}

#[test]
fn the_eighth_failure_in_a_row_disables_and_only_control_stops_it() {
    let r = rig();
    let exe = r.exe("dead", &format!("{}exit 3", r.counts()));
    for n in 1..=MAX_FAILURES {
        assert!(ensure(&r.paths, &exe).is_err(), "attempt {n}");
        assert_eq!(r.failures(), n);
        if n < MAX_FAILURES {
            assert!(control::enabled(&r.paths).is_ok(), "attempt {n}");
            r.pass_the_wait();
        }
    }
    assert!(control::enabled(&r.paths).is_err());
    assert!(ensure(&r.paths, &exe).is_err());
    assert_eq!(r.spawned(), MAX_FAILURES as usize);
}

#[test]
fn a_child_that_says_something_else_proves_nothing() {
    let r = rig();
    let exe = r.exe("odd", "printf 'READY v2\\n'; sleep 1");
    assert!(ensure(&r.paths, &exe).is_err());
    assert_eq!(r.failures(), 1);
}

#[test]
fn no_install_starts_nothing_and_creates_nothing() {
    let r = rig();
    let exe = r.exe("ok", &format!("{}printf 'READY v1\\n'", r.counts()));
    fs::remove_file(r.paths.file("control.json")).unwrap();
    assert!(ensure(&r.paths, &exe).is_err());
    assert_eq!(r.spawned(), 0);
    assert_eq!(fs::read_dir(&r.paths.data).unwrap().count(), 0);
}

#[test]
fn disabled_starts_nothing() {
    let r = rig();
    let exe = r.exe("ok", &format!("{}printf 'READY v1\\n'", r.counts()));
    control::disable(&r.paths).unwrap();
    assert!(ensure(&r.paths, &exe).is_err());
    assert_eq!(r.spawned(), 0);
}

#[test]
fn a_writer_of_this_binary_is_enough_and_ends_a_run_of_failures() {
    let r = rig();
    let exe = r.exe("ok", &format!("{}printf 'READY v1\\n'", r.counts()));
    Ledger::default().count(&r.paths, 1).unwrap();
    let _lock = r.hold_writer(&exe);
    assert_eq!(ensure(&r.paths, &exe), Ok(Ensured::Running));
    assert_eq!(r.spawned(), 0);
    assert_eq!(r.failures(), 0);
}

#[test]
fn a_writer_of_another_binary_that_will_not_leave_is_refused_and_counts_nothing() {
    let r = rig();
    let old = r.exe("old", "exit 0");
    let new = r.exe("new", &format!("{}printf 'READY v1\\n'", r.counts()));
    let _lock = r.hold_writer(&old);
    assert!(ensure(&r.paths, &new).is_err());
    assert_eq!(r.spawned(), 0);
    assert_eq!(r.failures(), 0);
    assert!(!r.paths.file("stop.request").exists(), "the request goes");
}

#[test]
fn a_writer_of_another_binary_is_asked_to_leave_and_replaced() {
    let r = rig();
    let old = r.exe("old", "exit 0");
    let new = r.exe("new", &format!("{}printf 'READY v1\\n'", r.counts()));
    let lock = r.hold_writer(&old);
    let stop = r.paths.file("stop.request");
    let leaves = std::thread::spawn(move || {
        while !stop.exists() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drop(lock); // the writer's poll saw the request
    });
    assert_eq!(ensure(&r.paths, &new), Ok(Ensured::Started));
    leaves.join().unwrap();
    assert_eq!(r.spawned(), 1);
    assert!(!r.paths.file("stop.request").exists());
}

#[test]
fn past_the_wait_for_ensure_lock_it_spawns_nothing_and_counts_nothing() {
    let r = rig();
    let exe = r.exe("ok", &format!("{}printf 'READY v1\\n'", r.counts()));
    let held = r.paths.lock_file("ensure.lock").unwrap();
    held.try_lock().unwrap();
    let start = Instant::now();
    assert!(ensure(&r.paths, &exe).is_err());
    assert!(start.elapsed() >= ENSURE_WAIT);
    assert_eq!(r.spawned(), 0);
    assert_eq!(r.failures(), 0);
}

/// A leftover request must not end the writer this start brings up.
#[test]
fn a_stop_request_left_behind_goes_before_a_spawn() {
    let r = rig();
    let exe = r.exe(
        "ok",
        "test ! -e \"$4\"/data/herdr-inbox/stop.request && printf 'READY v1\\n'",
    );
    fs::write(r.paths.file("stop.request"), b"").unwrap();
    assert_eq!(ensure(&r.paths, &exe), Ok(Ensured::Started));
}

/// An `--ensure` waiting behind an uninstall read `disabled: false` before it waited.
#[test]
fn control_is_read_again_once_ensure_lock_is_held() {
    let r = rig();
    let exe = r.exe("ok", &format!("{}printf 'READY v1\\n'", r.counts()));
    let held = r.paths.lock_file("ensure.lock").unwrap();
    held.try_lock().unwrap();
    let paths = r.paths.clone();
    let waiting = std::thread::spawn(move || ensure(&paths, &exe));
    std::thread::sleep(std::time::Duration::from_millis(300));
    control::disable(&r.paths).unwrap();
    drop(held);
    assert!(waiting.join().unwrap().is_err());
    assert_eq!(r.spawned(), 0);
}

/// Counted after the proof, a crash in between would count nothing, and the bound is gone.
#[test]
fn a_spawn_is_counted_before_it_happens() {
    let r = rig();
    let body =
        "grep -q '\"failures\":1' \"$4\"/data/herdr-inbox/start.json && printf 'READY v1\\n'";
    let exe = r.exe("sees-its-count", body);
    assert_eq!(ensure(&r.paths, &exe), Ok(Ensured::Started));
    assert_eq!(r.failures(), 0, "a proven start removes it");
}

/// A broken new binary in backoff must not cost the writer that works.
#[test]
fn a_deferred_start_leaves_the_running_writer_alone() {
    let r = rig();
    let old = r.exe("old", "exit 0");
    let new = r.exe("new", &format!("{}printf 'READY v1\\n'", r.counts()));
    let _lock = r.hold_writer(&old);
    Ledger::default().count(&r.paths, now_ms()).unwrap();
    assert!(ensure(&r.paths, &new).is_err());
    assert_eq!(r.spawned(), 0);
    assert!(
        !r.paths.file("stop.request").exists(),
        "it was never asked to leave"
    );
}

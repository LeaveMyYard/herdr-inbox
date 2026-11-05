//! Python `worker.ensure`: tests/test_delivery_worker.py's and
//! tests/test_stale_worker.py's.

use std::fs;

use super::super::*;
use super::fakes::{Fakes, wait_for};
use crate::worker::handover::{standby_path, write_holder};
use crate::worker::lock::hold;

fn workers_started(f: &Fakes) -> usize {
    f.calls()
        .iter()
        .filter(|l| l.starts_with("herdr-inbox|worker|"))
        .count()
}

#[test]
fn ensure_spawns_only_when_not_running() {
    let f = Fakes::new();
    let exe = f.logger("herdr-inbox");
    let env = f.env();
    assert!(ensure(&env, &exe));
    assert!(wait_for(|| workers_started(&f) == 1));
    assert!(
        env.state_dir().join("worker.log").exists(),
        "its stderr goes there"
    );
    let _held = hold(&env.lock_path()).unwrap().unwrap();
    write_holder(&env.lock_path(), env.revision);
    let stamp = stamp_path(&env.lock_path(), env.revision);
    fs::remove_file(stamp).unwrap(); // not the rate limit: the running worker says no
    assert!(!ensure(&env, &exe));
    let mut off = f.env();
    off.no_worker = true;
    drop(_held);
    assert!(!ensure(&off, &exe), "HERDR_INBOX_NO_WORKER");
    assert_eq!(workers_started(&f), 1);
}

#[test]
fn ensure_starts_a_standby_only_for_an_older_holder() {
    let f = Fakes::new();
    let exe = f.logger("herdr-inbox");
    let mut env = f.env();
    env.revision = 2;
    let lock = env.lock_path();
    let _held = hold(&lock).unwrap().unwrap();
    write_holder(&lock, 2);
    assert!(!ensure(&env, &exe), "the holder is this revision");
    write_holder(&lock, 1);
    assert!(
        ensure(&env, &exe),
        "an older holder: this revision's worker stands by"
    );
    assert!(wait_for(|| workers_started(&f) == 1));
    let other = std::os::unix::process::parent_id();
    let beat = unix_now();
    fs::write(standby_path(&lock), format!("2 {other} {beat:.3} -\n")).unwrap();
    fs::remove_file(stamp_path(&lock, 2)).unwrap();
    assert!(!ensure(&env, &exe), "one stands by already");
    assert_eq!(workers_started(&f), 1);
}

#[test]
fn ensure_starts_at_most_one_worker_per_revision_per_interval() {
    let f = Fakes::new();
    let exe = f.logger("herdr-inbox");
    let env = f.env();
    assert!(ensure(&env, &exe));
    assert!(
        !ensure(&env, &exe) && !ensure(&env, &exe),
        "the first may still be starting"
    );
    assert!(wait_for(|| workers_started(&f) == 1));
}

#[test]
fn a_worker_that_cannot_start_is_logged_never_written_over_the_callers_screen() {
    let dir = tempfile::tempdir().unwrap();
    let env = WorkerEnv {
        db: dir.path().join("inbox.db"),
        server: "/fake/herdr.sock".into(),
        revision: 1,
        version: "0.10.5".into(),
        waits: crate::worker::Waits::default(),
        spawn_env: Vec::new(),
        no_worker: false,
    };
    assert!(!ensure(&env, &dir.path().join("no-such-binary")));
    let log = fs::read_to_string(env.state_dir().join("worker.log")).unwrap();
    assert!(log.starts_with("inbox: delivery worker: "), "{log}");
}

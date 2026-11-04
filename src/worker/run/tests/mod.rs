//! The run loop's rules on a scripted world (Python tests/test_successor.py,
//! tests/test_stale_worker.py and tests/test_delivery_worker.py, the parts
//! that are the worker's life rather than its duties).

mod fake;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use fake::Fake;

use super::serve::{handed_over, newer_wanted};
use super::*;
use crate::worker::handover::{holder_path, standby_path};
use crate::worker::lock::is_running;
use crate::worker::{Waits, WorkerEnv};

fn env_in(dir: &std::path::Path) -> WorkerEnv {
    WorkerEnv {
        db: dir.join("inbox.db"),
        server: "/fake/herdr.sock".into(),
        revision: 1,
        version: "0.10.5".into(),
        waits: Waits {
            tick: Duration::ZERO,
            idle_exit: Duration::from_secs(3600),
            server_gone_exit: Duration::ZERO,
            ..Waits::default()
        },
        spawn_env: Vec::new(),
        no_worker: false,
    }
}

fn setup() -> (tempfile::TempDir, WorkerEnv, Fake) {
    let dir = tempfile::tempdir().unwrap();
    let env = env_in(dir.path());
    let fake = Fake::new(env.lock_path());
    (dir, env, fake)
}

fn standby(env: &WorkerEnv, revision: i64) {
    let other = std::os::unix::process::parent_id();
    let beat = unix_now();
    fs::write(
        standby_path(&env.lock_path()),
        format!("{revision} {other} {beat:.3} -\n"),
    )
    .unwrap();
}

#[test]
fn the_holder_hands_over_only_to_a_live_newer_standby() {
    let (_dir, env, _) = setup();
    assert!(!handed_over(&env));
    standby(&env, env.revision);
    assert!(!handed_over(&env), "not newer");
    standby(&env, env.revision + 1);
    assert!(handed_over(&env));
}

/// Python test_a_marker_asks_the_holder_for_a_newer_worker, its five cases.
#[test]
fn a_marker_asks_the_holder_for_a_newer_worker() {
    let (_dir, env, mut w) = setup();
    assert!(!newer_wanted(&env, &mut w), "none");
    (w.wanted, w.stamp) = (Some(2), Some(1));
    assert!(newer_wanted(&env, &mut w), "newer");
    (w.wanted, w.stamp) = (Some(2), None);
    assert!(newer_wanted(&env, &mut w), "newer, on a legacy stamp");
    (w.wanted, w.stamp) = (Some(1), None);
    assert!(
        !newer_wanted(&env, &mut w),
        "same revision on a legacy stamp"
    );
    (w.wanted, w.stamp) = (Some(2), Some(2));
    assert!(!newer_wanted(&env, &mut w), "db already there");
    (w.wanted, w.stamp) = (Some(2), Some(1));
    standby(&env, 2);
    assert!(!newer_wanted(&env, &mut w), "standby waiting");
}

#[test]
fn a_worker_removes_its_holder_record_when_it_lets_go() {
    let (_dir, env, mut w) = setup();
    w.gone = [true, true].into();
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(
        w.calls,
        [format!(
            "begin 1 {} {}",
            std::process::id(),
            crate::worker::handover::start_time(std::process::id())
        )],
        "written while it held the lock"
    );
    assert!(
        !holder_path(&env.lock_path()).exists(),
        "gone with the lock"
    );
    assert!(!is_running(&env.lock_path()));
}

#[test]
fn another_worker_of_this_revision_holding_the_lock_ends_the_run() {
    let (_dir, env, mut w) = setup();
    let _other = crate::worker::lock::hold(&env.lock_path())
        .unwrap()
        .unwrap();
    assert_eq!(run(&env, &mut w), 0);
    assert!(w.calls.is_empty() && w.ticks == 0, "never served");
}

#[test]
fn a_newer_worker_stands_by_for_an_older_holder_then_serves() {
    let (_dir, mut env, mut w) = setup();
    env.revision = 2;
    let other = crate::worker::lock::hold(&env.lock_path())
        .unwrap()
        .unwrap();
    // the older holder's record (this test process stands in for it)
    crate::worker::handover::write_holder(&env.lock_path(), 1);
    w.other = Some((other, 5));
    w.stop_after = Some(1);
    assert_eq!(run(&env, &mut w), 0);
    assert!(
        w.calls[0].starts_with("begin 2 "),
        "{:?}: it took the lock as revision 2",
        w.calls
    );
    assert_eq!(w.ticks, 1);
    assert!(!standby_path(&env.lock_path()).exists());
}

#[test]
fn the_worker_exits_when_its_server_is_gone() {
    let (_dir, env, mut w) = setup();
    w.gone = [true, true].into();
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(w.ticks, 0, "no tick without the server");
    assert!(!is_running(&env.lock_path()));
}

#[test]
fn the_worker_exits_once_the_server_goes_away() {
    let (_dir, env, mut w) = setup();
    w.stop_after = Some(3);
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(w.ticks, 3);
    assert!(!is_running(&env.lock_path()));
}

/// Python review M2: a lone "not running" reading is no exit, even with no
/// grace: that tick is skipped, and serving goes on once herdr answers.
#[test]
fn a_moment_without_the_server_does_not_end_the_worker() {
    let (_dir, env, mut w) = setup();
    w.gone = [true, false, true, false, true, false].into();
    w.stop_after = Some(7);
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(
        (w.ticks, w.gone_checks),
        (7, 10),
        "the 3 gone readings skipped their tick"
    );
}

#[test]
fn gone_for_the_whole_grace_ends_the_worker() {
    let (_dir, mut env, mut w) = setup();
    env.waits.tick = Duration::from_millis(10);
    env.waits.server_gone_exit = Duration::from_millis(200);
    w.gone = std::iter::repeat_n(true, 100).collect();
    assert_eq!(run(&env, &mut w), 0);
    assert!(w.t >= 0.2 && w.t < 0.25, "{}", w.t);
    assert_eq!(w.ticks, 0);
}

/// Python test_the_running_worker_hands_over_to_rust_and_exits and
/// test_the_worker_hands_over_when_the_disk_version_differs_from_its_own.
#[test]
fn the_worker_hands_over_when_the_disk_version_differs_from_its_own() {
    let (_dir, env, mut w) = setup();
    w.installed = Some("0.11.0".into());
    w.current = Some(PathBuf::from("/install/0.11.0"));
    w.successor_starts = true;
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(w.ticks, 1);
    assert_eq!(
        w.calls[1], "start_successor Some(\"/install/0.11.0\") held=false",
        "released first"
    );
    assert!(!is_running(&env.lock_path()));
}

#[test]
fn the_same_version_on_disk_or_none_is_no_handover() {
    let (_dir, env, mut w) = setup();
    w.installed = Some(env.version.clone());
    w.stop_after = Some(3);
    run(&env, &mut w);
    w.installed = None;
    w.stop_after = Some(6);
    run(&env, &mut w);
    assert!(
        w.calls.iter().all(|c| !c.starts_with("start_successor")),
        "{:?}",
        w.calls
    );
}

#[test]
fn no_handover_loop_when_no_successor_can_start() {
    let (_dir, env, mut w) = setup();
    w.installed = Some("0.11.0".into());
    w.stop_after = Some(5);
    let lock = env.lock_path();
    assert_eq!(run(&env, &mut w), 0);
    let tries = w
        .calls
        .iter()
        .filter(|c| c.starts_with("start_successor"))
        .count();
    assert_eq!(
        (tries, w.ticks),
        (1, 5),
        "tried once, kept serving with this code"
    );
    assert!(
        !holder_path(&lock).exists(),
        "re-held, re-recorded, then cleared on exit"
    );
}

/// Python test_a_worker_that_idles_out_with_new_work_starts_a_successor.
#[test]
fn a_worker_that_idles_out_with_new_work_starts_a_successor() {
    let (_dir, mut env, mut w) = setup();
    env.waits.tick = Duration::from_secs(5);
    env.waits.idle_exit = Duration::from_secs(600);
    w.ticked = Ticked::default();
    w.pending_now = 1;
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(
        w.calls[1..],
        ["pending_now held=false", "ensure"],
        "looked once the lock was free"
    );
    assert!(w.t >= 600.0 && w.t < 610.0, "{}", w.t);
    let (_dir, env, mut w) = setup();
    let mut env = env;
    env.waits.idle_exit = Duration::ZERO;
    w.ticked = Ticked::default();
    run(&env, &mut w);
    assert_eq!(
        w.calls[1..],
        ["pending_now held=false"],
        "nothing new: no successor"
    );
    env.waits.idle_exit = Duration::from_secs(1);
    w.ticked.keep_alive = true;
    w.stop_after = Some(w.ticks + 50);
    w.calls.clear();
    env.waits.tick = Duration::from_secs(1);
    run(&env, &mut w);
    assert!(
        !w.calls.iter().any(|c| c.starts_with("pending_now")),
        "kept alive: no idle exit"
    );
}

#[test]
fn a_newer_standby_ends_the_holders_service() {
    let (_dir, env, mut w) = setup();
    w.stop_after = Some(1000);
    standby(&env, 2);
    assert_eq!(run(&env, &mut w), 0);
    assert_eq!(w.ticks, 0, "handed over before ticking");
    assert!(!is_running(&env.lock_path()));
}

#[test]
fn a_marker_starts_the_listed_installs_worker_at_most_every_minute() {
    let (_dir, mut env, mut w) = setup();
    env.waits.tick = Duration::from_secs(5);
    (w.wanted, w.listed, w.current) = (Some(2), Some("/listed".into()), Some("/own".into()));
    w.stop_after = Some(30);
    run(&env, &mut w);
    let spawns: Vec<_> = w
        .calls
        .iter()
        .filter(|c| c.starts_with("spawn_newer"))
        .collect();
    assert_eq!(
        spawns,
        [
            "spawn_newer Some(\"/listed\") at 0",
            "spawn_newer Some(\"/listed\") at 60",
            "spawn_newer Some(\"/listed\") at 120"
        ]
    );
    w.listed = None;
    w.calls.clear();
    w.stop_after = Some(w.ticks + 1);
    run(&env, &mut w);
    assert!(
        w.calls
            .iter()
            .any(|c| c.starts_with("spawn_newer Some(\"/own\")")),
        "{:?}",
        w.calls
    );
}

//! The watcher's lock and owner files, with real time (two real watchers).

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use herdr_agent_kit::plugin_env::LockFile;
use herdr_inbox::hook::payload::Payload;
use herdr_inbox::rewake::{SystemClock, run};

use crate::common::A;
use crate::endable::{EndWatchers, EndableClock, join_within};
use crate::world::{FakeClock, World};

fn lock_of(w: &World, sid: &str) -> LockFile {
    LockFile::new(w.dir.path().join("rewake").join(format!("{sid}.lock")))
}

fn rewrite_owner(w: &World) {
    std::fs::write(
        w.dir.path().join("rewake").join(format!("{A}.owner")),
        "newer",
    )
    .unwrap();
}

#[test]
fn a_watcher_holds_its_lock_and_hands_it_to_the_one_that_supersedes_it() {
    // review #67 M1 (the reviewer's probe): the lock is what the worker's watcher_alive reads
    let w = World::new();
    let lock = lock_of(&w, A);
    let ended = AtomicBool::new(false);
    std::thread::scope(|s| {
        let _end = EndWatchers(&w, &ended);
        let ended = &ended;
        let t0 = Instant::now();
        let w = &w;
        let first = s.spawn(move || {
            let c = EndableClock::new(ended);
            (
                run(&w.watcher(&c, Some("w1:p1")), &w.payload()),
                t0.elapsed(),
            )
        });
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            lock.try_exclusive().unwrap().is_none(),
            "watcher 1 holds its lock while watching"
        );
        let second = s.spawn(move || {
            let c = EndableClock::new(ended);
            run(&w.watcher(&c, Some("w1:p1")), &w.payload())
        });
        let (r1, t1) = join_within(first, Duration::from_secs(6), ended);
        assert!(
            r1.is_none() && t1 < Duration::from_secs(4),
            "superseded within a poll: {t1:?}"
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            lock.try_exclusive().unwrap().is_none(),
            "watcher 2 holds the lock now"
        );
        assert!(!second.is_finished());
        rewrite_owner(w);
        assert!(join_within(second, Duration::from_secs(6), ended).is_none());
    });
    assert!(
        lock.try_exclusive().unwrap().is_some(),
        "both gone: the lock is free"
    );
}

#[test]
fn the_lock_wait_gives_up_when_superseded_or_after_lock_wait() {
    // review #67 L1: another process holds the lock
    let w = World::new();
    std::fs::create_dir_all(w.dir.path().join("rewake")).unwrap();
    let held = lock_of(&w, A)
        .try_exclusive()
        .unwrap()
        .expect("the test holds it");
    let ended = AtomicBool::new(false);
    std::thread::scope(|s| {
        let _end = EndWatchers(&w, &ended);
        let ended = &ended;
        let w = &w;
        let t0 = Instant::now();
        let waiting = s.spawn(move || {
            let c = EndableClock::new(ended);
            run(&w.watcher(&c, Some("w1:p1")), &w.payload())
        });
        std::thread::sleep(Duration::from_millis(500));
        rewrite_owner(w);
        assert!(join_within(waiting, Duration::from_secs(6), ended).is_none());
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "gave up on losing ownership: {:?}",
            t0.elapsed()
        );
    });
    let t0 = Instant::now();
    let c = SystemClock::new();
    assert!(run(&w.watcher(&c, Some("w1:p1")), &w.payload()).is_none());
    let took = t0.elapsed();
    assert!(
        took >= Duration::from_millis(4900) && took < Duration::from_secs(8),
        "LOCK_WAIT: {took:?}"
    );
    drop(held);
}

#[test]
fn a_bad_session_id_never_names_a_file() {
    // Python test_bridge_inbox::test_rewake_never_names_a_file_after_a_bad_session_id
    let w = World::new();
    for bad in [
        "../../x",
        "*",
        "a/b",
        "55555555-5555-4555-8555-555555555555 ",
        "agent-a1",
    ] {
        let payload = Payload {
            session_id: Some(bad.into()),
            ..Payload::default()
        };
        assert!(
            run(
                &w.watcher(&FakeClock::new(|_, _| {}), Some("w1:p1")),
                &payload
            )
            .is_none(),
            "{bad}"
        );
    }
    assert!(!w.dir.path().join("rewake").exists());
}

#[test]
fn the_files_use_the_canonical_id() {
    // Python test_bridge_inbox::test_rewake_files_use_the_canonical_id
    let w = World::new();
    let payload = Payload {
        session_id: Some(A.to_uppercase()),
        ..Payload::default()
    };
    let clock = FakeClock::new(|n, t| {
        if n == 1 {
            t.set(1e9)
        }
    });
    assert!(run(&w.watcher(&clock, Some("w1:p1")), &payload).is_none());
    let mut names: Vec<String> = std::fs::read_dir(w.dir.path().join("rewake"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, [format!("{A}.lock"), format!("{A}.owner")]);
}

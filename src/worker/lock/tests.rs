use std::time::{Duration, Instant};

use super::*;

/// Python test_delivery_worker::test_the_worker_is_a_singleton.
#[test]
fn the_worker_is_a_singleton() {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("state").join("worker-x.lock");
    assert!(!is_running(&lock));
    let held = hold(&lock)
        .unwrap()
        .expect("free: taken, its directory created");
    assert!(is_running(&lock));
    assert!(
        hold(&lock).unwrap().is_none(),
        "a second worker cannot take it"
    );
    let started = Instant::now();
    assert!(hold_within(&lock).unwrap().is_none());
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "retried ~200 ms first"
    );
    drop(held);
    assert!(!is_running(&lock));
    assert!(
        hold_within(&lock).unwrap().is_some(),
        "released with its holder"
    );
}

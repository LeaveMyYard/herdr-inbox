use std::path::Path;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn the_lock_sits_next_to_the_database() {
    assert_eq!(
        lock_path(Path::new("/s/inbox.db")),
        Path::new("/s/inbox.db.markers.lock")
    );
}

#[test]
fn a_held_lock_is_waited_for_then_given_up_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    let mut warned = Vec::new();
    let held = marker_lock(&db, Duration::ZERO, &mut |m| warned.push(m.to_string())).unwrap();
    assert!(held.is_some() && warned.is_empty());
    let started = Instant::now();
    let second = marker_lock(&db, Duration::from_millis(150), &mut |m| {
        warned.push(m.to_string())
    });
    assert!(second.unwrap().is_none());
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert_eq!(warned, vec![BUSY.to_string()]);
    drop(held);
    assert!(
        marker_lock(&db, Duration::ZERO, &mut |_| {})
            .unwrap()
            .is_some()
    );
}

use std::fs;
use std::time::Duration;

use super::{Ledger, backoff, clear};
use crate::app::writer::files::Paths;

fn paths() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    fs::create_dir_all(&paths.data).unwrap();
    (dir, paths)
}

#[test]
fn the_backoff_doubles_up_to_160_seconds() {
    let seconds: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
    assert_eq!(seconds, [5, 10, 20, 40, 80, 160, 160, 160]);
}

#[test]
fn absent_is_no_failures_and_a_count_defers_the_next_spawn() {
    let (_dir, paths) = paths();
    let none = Ledger::read(&paths).unwrap();
    assert_eq!(none.failures, 0);
    assert_eq!(none.deferred(1_000), None);
    let first = none.count(&paths, 1_000).unwrap();
    assert_eq!(first.failures, 1);
    assert_eq!(first.retry_at_ms, 6_000);
    assert_eq!(Ledger::read(&paths).unwrap(), first);
    assert_eq!(first.deferred(2_000), Some(Duration::from_secs(4)));
    assert_eq!(first.deferred(6_000), None);
    clear(&paths);
    assert_eq!(Ledger::read(&paths).unwrap().failures, 0);
}

/// A clock that went back must not defer for ever.
#[test]
fn a_date_past_the_longest_backoff_counts_as_passed() {
    let (_dir, paths) = paths();
    let ledger = Ledger::default().count(&paths, 1_000_000).unwrap();
    assert_eq!(ledger.deferred(0), None);
}

/// Forgetting the count would drop the bound.
#[test]
fn a_ledger_that_cannot_be_read_stops_starts() {
    let (_dir, paths) = paths();
    fs::write(paths.file("start.json"), b"not json").unwrap();
    assert!(Ledger::read(&paths).is_err());
}

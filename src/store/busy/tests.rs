use std::cell::Cell;
use std::time::{Duration, Instant};

use rusqlite::ffi;

use super::*;

fn failure(extended: i32) -> Error {
    Error::SqliteFailure(ffi::Error::new(extended), Some("database is locked".into()))
}

/// A `work` that fails with each code in turn, then returns how many times it ran.
fn raising(
    codes: Vec<i32>,
    calls: &Cell<usize>,
) -> impl FnMut(&Connection) -> rusqlite::Result<usize> + '_ {
    move |_| {
        calls.set(calls.get() + 1);
        match codes.get(calls.get() - 1) {
            Some(&code) => Err(failure(code)),
            None => Ok(calls.get()),
        }
    }
}

#[test]
fn bounded_retries_extended_busy_codes() {
    // Python test_hook::test_bounded_retries_extended_busy_codes: BUSY, BUSY_RECOVERY (261),
    // BUSY_SNAPSHOT (517), LOCKED, LOCKED_SHAREDCACHE (262)
    for code in [5, 261, 517, 6, 262] {
        let calls = Cell::new(0);
        let got = bounded(
            Connection::open_in_memory,
            raising(vec![code], &calls),
            Duration::from_secs(5),
        );
        assert_eq!(got.unwrap(), 2, "{code}");
    }
}

#[test]
fn bounded_gives_up_on_other_errors_and_at_the_deadline() {
    // Python test_hook::test_bounded_gives_up_on_other_errors_and_at_the_deadline
    let calls = Cell::new(0);
    let got = bounded(
        Connection::open_in_memory,
        raising(vec![1], &calls),
        Duration::from_secs(5),
    );
    assert!(
        got.is_err() && calls.get() == 1,
        "SQLITE_ERROR is not busy: returned at once"
    );
    let calls = Cell::new(0);
    let started = Instant::now();
    let got = bounded(
        Connection::open_in_memory,
        raising(vec![5; 10_000], &calls),
        Duration::from_millis(100),
    );
    assert!(got.is_err() && is_busy(&got.unwrap_err()));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(calls.get() > 1, "retried until the deadline");
}

#[test]
fn bounded_opens_a_fresh_connection_for_every_attempt() {
    // Python test_hook::test_bounded_closes_every_connection_it_opens: each attempt's
    // connection is owned by that attempt and dropped (closed) at its end
    let (opened, calls) = (Cell::new(0), Cell::new(0));
    let connect = || {
        opened.set(opened.get() + 1);
        Connection::open_in_memory()
    };
    bounded(connect, raising(vec![5, 5], &calls), Duration::from_secs(5)).unwrap();
    assert_eq!((opened.get(), calls.get()), (3, 3));
}

#[test]
fn a_failed_connect_is_retried_like_failed_work() {
    let tries = Cell::new(0);
    let connect = || {
        tries.set(tries.get() + 1);
        if tries.get() == 1 {
            Err(failure(5))
        } else {
            Connection::open_in_memory()
        }
    };
    assert_eq!(
        bounded(connect, |_| Ok(7), Duration::from_secs(5)).unwrap(),
        7
    );
    assert!(!is_busy(&Error::QueryReturnedNoRows));
}

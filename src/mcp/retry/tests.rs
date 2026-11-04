use std::cell::{Cell, RefCell};
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::mcp::receipts;

/// Runs `busy_safe` over `outcomes` (one per attempt); returns its answer, the
/// attempts made, the worker starts asked for and the waits slept.
fn run(
    outcomes: Vec<Result<Value, Failure>>,
    backoff: &Backoff,
) -> (Result<Value, String>, usize, usize, Vec<Duration>) {
    let outcomes = RefCell::new(outcomes.into_iter());
    let attempts = Cell::new(0);
    let starts = Cell::new(0);
    let slept = RefCell::new(Vec::new());
    let got = busy_safe(
        || {
            attempts.set(attempts.get() + 1);
            outcomes
                .borrow_mut()
                .next()
                .expect("no more attempts expected")
        },
        backoff,
        &|| starts.set(starts.get() + 1),
        &|d| slept.borrow_mut().push(d),
    );
    (got, attempts.get(), starts.get(), slept.into_inner())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn short() -> Backoff {
    Backoff {
        busy: vec![ms(1), ms(2), ms(3)],
        not_ready: vec![ms(10), ms(20)],
    }
}

#[test]
fn a_result_is_returned_at_once() {
    let (got, attempts, starts, slept) = run(vec![Ok(json!({"a": 1}))], &short());
    assert_eq!((got, attempts, starts), (Ok(json!({"a": 1})), 1, 0));
    assert!(slept.is_empty());
}

#[test]
fn a_busy_database_is_retried_then_answered_busy() {
    let busy = || Err(Failure::Busy);
    let (got, attempts, _, slept) = run(vec![busy(), busy(), busy(), busy()], &short());
    assert_eq!(got, Ok(receipts::busy()));
    assert_eq!(attempts, 4);
    assert_eq!(slept, [ms(1), ms(2), ms(3)]);
    let (got, attempts, _, _) = run(vec![busy(), Ok(json!(1))], &short());
    assert_eq!((got, attempts), (Ok(json!(1)), 2));
}

#[test]
fn not_ready_asks_for_the_worker_every_attempt_then_answers_not_migrated() {
    let nr = || Err(Failure::NotReady);
    let (got, attempts, starts, slept) = run(vec![nr(), nr(), nr()], &short());
    assert_eq!(got, Ok(receipts::not_migrated()));
    // Python's test: 3 attempts and 3 worker starts with two waits (#50)
    assert_eq!((attempts, starts), (3, 3));
    assert_eq!(slept, [ms(10), ms(20)]);
}

#[test]
fn a_worker_that_migrates_in_time_lets_the_call_through() {
    let (got, attempts, starts, _) =
        run(vec![Err(Failure::NotReady), Ok(json!("filed"))], &short());
    assert_eq!((got, attempts, starts), (Ok(json!("filed")), 2, 1));
}

#[test]
fn busy_during_the_not_ready_waits_fails_the_call_as_in_python() {
    let (got, attempts, _, _) = run(vec![Err(Failure::NotReady), Err(Failure::Busy)], &short());
    assert_eq!((got, attempts), (Err("database is locked".into()), 2));
}

#[test]
fn other_errors_fail_the_call() {
    let (got, attempts, _, _) = run(vec![Err(Failure::Error("boom".into()))], &short());
    assert_eq!((got, attempts), (Err("boom".into()), 1));
}

#[test]
fn the_not_ready_waits_are_spent_once_per_call() {
    // a busy attempt, then not ready: the not-ready waits run once, as Python's one iterator
    let (got, attempts, starts, slept) = run(
        vec![
            Err(Failure::Busy),
            Err(Failure::NotReady),
            Err(Failure::NotReady),
            Err(Failure::NotReady),
        ],
        &short(),
    );
    assert_eq!(got, Ok(receipts::not_migrated()));
    assert_eq!((attempts, starts), (4, 3));
    assert_eq!(slept, [ms(1), ms(10), ms(20)]);
}

#[test]
fn the_default_waits_are_pythons() {
    let b = Backoff::default();
    let secs: Vec<f64> = b.busy.iter().map(Duration::as_secs_f64).collect();
    assert_eq!(secs, [0.3, 1.0, 2.0]);
    let secs: Vec<f64> = b.not_ready.iter().map(Duration::as_secs_f64).collect();
    assert_eq!(secs, [0.3, 1.0, 2.0, 3.0, 3.0]);
    assert_eq!(TOOL_BUSY_TIMEOUT, Duration::from_secs(3));
}

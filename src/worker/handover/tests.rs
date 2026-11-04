//! Python tests/test_stale_worker.py, "the holder and the standby".

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::worker::spawn::unix_now;

fn me() -> u32 {
    std::process::id()
}

/// A live process that is not this one.
fn other() -> u32 {
    std::os::unix::process::parent_id()
}

fn lock_in(dir: &Path) -> PathBuf {
    dir.join("worker-x.lock")
}

/// Time that passes only when slept: a wait under a mutant ends at its max
/// at once instead of hanging.
struct FakeClock<F: FnMut(f64) -> Option<&'static str>> {
    t: f64,
    take: F,
}

impl<F: FnMut(f64) -> Option<&'static str>> Standby for FakeClock<F> {
    type Held = &'static str;
    fn take(&mut self) -> Option<&'static str> {
        (self.take)(self.t)
    }
    fn monotonic(&self) -> f64 {
        self.t
    }
    fn unix(&self) -> f64 {
        unix_now()
    }
    fn sleep(&mut self, d: Duration) {
        self.t += d.as_secs_f64();
    }
}

fn clock<F: FnMut(f64) -> Option<&'static str>>(take: F) -> FakeClock<F> {
    FakeClock { t: 0.0, take }
}

#[test]
fn holder_revision_is_the_live_holders_or_1() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    assert_eq!(holder_revision(&lk), 1, "a worker from before the record");
    write_holder(&lk, 3);
    assert_eq!(holder_revision(&lk), 3);
    let record = holder_path(&lk);
    assert_eq!(record, dir.path().join("worker-x.lock.holder"));
    fs::write(&record, format!("3 999999999 {}\n", start_time(me()))).unwrap();
    assert_eq!(holder_revision(&lk), 1, "its process is gone");
    fs::write(&record, format!("3 {} 12345\n", me())).unwrap();
    assert_eq!(
        holder_revision(&lk),
        1,
        "its pid is another (later) process's now"
    );
    fs::write(&record, format!("3 {} -\n", me())).unwrap();
    assert_eq!(
        holder_revision(&lk),
        3,
        "an unreadable start time is trusted"
    );
    fs::write(&record, "garbage\n").unwrap();
    assert_eq!(holder_revision(&lk), 1);
    assert!(
        !["", "-"].contains(&start_time(me()).as_str()),
        "readable here"
    );
}

#[test]
fn the_holder_record_is_pythons() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    write_holder(&lk, 2);
    let text = fs::read_to_string(holder_path(&lk)).unwrap();
    assert_eq!(text, format!("2 {} {}\n", me(), start_time(me())));
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["worker-x.lock.holder"], "no temporary file left");
}

#[test]
fn a_holder_clears_its_own_record_only() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    write_holder(&lk, 3);
    clear_holder(&lk);
    assert!(!holder_path(&lk).exists());
    fs::write(holder_path(&lk), format!("3 {} -\n", other())).unwrap();
    clear_holder(&lk);
    assert!(holder_path(&lk).exists(), "another process's");
}

/// Python test_a_standby_must_be_the_process_that_wrote_it_and_not_from_the_future.
#[test]
fn a_standby_must_be_the_process_that_wrote_it_and_not_from_the_future() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let path = standby_path(&lk);
    let (o, started) = (other(), start_time(other()));
    let now = unix_now();
    fs::write(&path, format!("2 {o} {now:.3} {started}\n")).unwrap();
    assert_eq!(live_standby(&lk, now), Some((2, o)));
    fs::write(&path, format!("2 {o} {now:.3} not-its-start\n")).unwrap();
    assert_eq!(
        live_standby(&lk, now),
        None,
        "another process now has that pid"
    );
    let ahead = now + STANDBY_FRESH + 1.0;
    fs::write(&path, format!("2 {o} {ahead:.3} {started}\n")).unwrap();
    assert_eq!(live_standby(&lk, now), None, "a heartbeat from the future");
    let skew = now + STANDBY_FRESH - 1.0;
    fs::write(&path, format!("2 {o} {skew:.3} {started}\n")).unwrap();
    assert_eq!(live_standby(&lk, now), Some((2, o)), "within the skew");
}

#[test]
fn a_standby_counts_while_alive_and_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let path = standby_path(&lk);
    let now = unix_now();
    fs::write(&path, format!("2 {} {now:.3} -\n", me())).unwrap();
    assert_eq!(live_standby(&lk, now), Some((2, me())));
    let old = now - STANDBY_FRESH - 1.0;
    fs::write(&path, format!("2 {} {old:.3} -\n", me())).unwrap();
    assert_eq!(live_standby(&lk, now), None, "its heartbeat stopped");
    fs::write(&path, format!("2 999999999 {now:.3} -\n")).unwrap();
    assert_eq!(live_standby(&lk, now), None, "its process is gone");
    fs::write(&path, "2 1 x -\n").unwrap();
    assert_eq!(live_standby(&lk, now), None, "unreadable");
}

#[test]
fn a_standby_writes_its_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let record = standby_path(&lk);
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut world = clock(|_| {
        if let Ok(text) = fs::read_to_string(&record) {
            seen.push(text.split_whitespace().map(String::from).collect());
        }
        (seen.len() == 3).then_some("LOCK")
    });
    assert_eq!(
        wait_as_standby(&lk, 2, STANDBY_MAX, &mut world),
        Some("LOCK")
    );
    let last = seen.last().unwrap();
    let (pid, started) = (me().to_string(), start_time(me()));
    assert_eq!(
        (last[0].as_str(), last[1].as_str(), last[3].as_str()),
        ("2", pid.as_str(), started.as_str())
    );
}

#[test]
fn a_standby_takes_the_lock_once_free_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let mut tries = 0;
    let mut world = clock(|_| {
        tries += 1;
        (tries == 30).then_some("LOCK")
    });
    assert_eq!(
        wait_as_standby(&lk, 2, STANDBY_MAX, &mut world),
        Some("LOCK")
    );
    assert!(!standby_path(&lk).exists(), "its record goes with it");
}

#[test]
fn a_standby_gives_up_after_its_max() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let mut world = clock(|_| None);
    assert_eq!(
        wait_as_standby(&lk, 2, Duration::from_secs(30), &mut world),
        None
    );
    assert!((30.0..31.0).contains(&world.t), "{}", world.t);
    assert!(!standby_path(&lk).exists());
}

#[test]
fn one_standby_is_enough() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let o = other();
    fs::write(standby_path(&lk), format!("2 {o} {:.3} -\n", unix_now())).unwrap();
    let mut world = clock(|_| None);
    assert_eq!(wait_as_standby(&lk, 2, STANDBY_MAX, &mut world), None);
    assert!(world.t < 1.0, "left at once");
    let kept = fs::read_to_string(standby_path(&lk)).unwrap();
    assert_eq!(
        kept.split_whitespace().nth(1),
        Some(o.to_string().as_str()),
        "never removes another's record"
    );
    // a lower revision waiting is no reason to leave
    fs::write(standby_path(&lk), format!("1 {o} {:.3} -\n", unix_now())).unwrap();
    let mut world = clock(|t| (t > 2.0).then_some("LOCK"));
    assert_eq!(
        wait_as_standby(&lk, 2, STANDBY_MAX, &mut world),
        Some("LOCK")
    );
}

/// Python #75 review L1: a second newer worker won the lock in the handover window.
#[test]
fn a_standby_leaves_once_a_holder_of_its_revision_has_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let lk = lock_in(dir.path());
    let holder = holder_path(&lk);
    let mut tries = 0;
    let mut world = clock(|_| {
        tries += 1;
        if tries == 15 {
            let o = other();
            fs::write(&holder, format!("2 {o} {}\n", start_time(o))).unwrap();
        }
        None
    });
    assert_eq!(wait_as_standby(&lk, 2, STANDBY_MAX, &mut world), None);
    assert!(world.t < 3.0, "within a heartbeat or two: {}", world.t);
    assert!(!standby_path(&lk).exists());
}

/// Python test_start_time_never_raises: a process that cannot be read is `-`.
#[test]
fn start_time_never_fails() {
    assert_eq!(start_time(999_999_999), "-");
}

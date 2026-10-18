use std::time::{Duration, SystemTime};

use herdr_inbox::clock::iso_seconds;
use herdr_inbox::store::liveness::{LIVENESS_EVERY, Stamp, touch};
use rusqlite::OptionalExtension;

use crate::common::*;

fn stamped(path: &std::path::Path, column: &str) -> Option<String> {
    open(path)
        .query_row(
            &format!("SELECT {column} FROM sessions WHERE session_id = ?"),
            [A],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten()
}

#[test]
fn a_stamp_is_written_at_most_once_a_minute_in_its_own_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let t0 = SystemTime::now();
    touch(&path, A, Stamp::HookSeen, t0);
    assert_eq!(stamped(&path, "hook_seen_at"), Some(iso_seconds(t0)));
    assert_eq!(
        stamped(&path, "watch_seen_at"),
        None,
        "the watcher's column is its own"
    );
    touch(&path, A, Stamp::HookSeen, t0 + Duration::from_secs(30));
    assert_eq!(
        stamped(&path, "hook_seen_at"),
        Some(iso_seconds(t0)),
        "fresh enough: no write"
    );
    let later = t0 + LIVENESS_EVERY + Duration::from_secs(1);
    touch(&path, A, Stamp::HookSeen, later);
    assert_eq!(stamped(&path, "hook_seen_at"), Some(iso_seconds(later)));
    touch(&path, A, Stamp::WatchSeen, later);
    assert_eq!(stamped(&path, "watch_seen_at"), Some(iso_seconds(later)));
}

#[test]
fn a_missing_database_is_never_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    touch(&path, A, Stamp::HookSeen, SystemTime::now());
    assert!(!path.exists());
}

#[test]
fn touch_gives_up_quickly_on_a_locked_db() {
    // Python test_hook::test_touch_gives_up_quickly_on_a_locked_db: the read-only look
    // (EXCLUSIVE: readers blocked) and the write (IMMEDIATE: the look finds no stamp, the
    // write is blocked) are both bounded by wall clock
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    for mode in ["EXCLUSIVE", "IMMEDIATE"] {
        let lock = locked(&path, mode);
        let ((), took) = timed(|| touch(&path, A, Stamp::HookSeen, SystemTime::now()));
        lock.execute_batch("ROLLBACK").unwrap();
        assert!(took < LOOKUP_BUDGET, "{mode} {took:?}");
    }
    assert_eq!(
        stamped(&path, "hook_seen_at"),
        None,
        "nothing was stamped while locked"
    );
}

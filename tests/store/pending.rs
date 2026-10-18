use herdr_inbox::store::pending::pending_lines;
use rusqlite::params;

use crate::common::*;

#[test]
fn open_and_answered_unseen_questions_of_this_session_only() {
    // the rows of Python test_hook::test_compact_lists_this_sessions_pending_questions
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let db = open(&path);
    ask(&db, A, "Which  region?\n(eu or us)");
    let answered = ask(&db, A, "Squash or merge?");
    answer(&db, answered, "squash");
    let seen = ask(&db, A, "Seen already?");
    answer(&db, seen, "yes");
    db.execute(
        "UPDATE questions SET acknowledged_at = 'x' WHERE id = ?",
        params![seen],
    )
    .unwrap();
    let closed = ask(&db, A, "Withdrawn?");
    db.execute(
        "UPDATE questions SET status = 'withdrawn' WHERE id = ?",
        params![closed],
    )
    .unwrap();
    ask(&db, B, "Other session question?");
    let null_answer = ask(&db, A, "No answer text?");
    db.execute(
        "UPDATE questions SET status = 'answered' WHERE id = ?",
        params![null_answer],
    )
    .unwrap();
    assert_eq!(
        pending_lines(&path, A),
        [
            "Q1 (open): Which region? (eu or us)",
            "Q2 (answered, not yet seen): Squash or merge? -> squash",
            "Q6 (answered, not yet seen): No answer text? -> None",
        ]
    );
    assert!(pending_lines(&path, "nobody").is_empty());
}

#[test]
fn a_missing_locked_or_broken_database_lists_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(pending_lines(&dir.path().join("none.db"), A).is_empty());
    let path = migrated(dir.path());
    ask(&open(&path), A, "Which region?");
    let lock = locked(&path, "EXCLUSIVE");
    let (lines, took) = timed(|| pending_lines(&path, A));
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(lines.is_empty() && took < LOOKUP_BUDGET, "{took:?}");
    let broken = dir.path().join("broken.db");
    std::fs::write(&broken, "not a database").unwrap();
    assert!(pending_lines(&broken, A).is_empty());
}

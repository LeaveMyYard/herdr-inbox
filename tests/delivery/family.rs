use herdr_inbox::delivery::forward_family;

use crate::common::*;
use crate::transcripts::*;

#[test]
fn the_family_is_directional_never_an_ancestor() {
    // Python test_older_conversation::test_older_conversation_is_directional
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    p.write(
        OLD,
        &[
            user(OLD, "2026-10-16T11:00:00Z", "work"),
            continued_in(OLD, NEW, Some("2026-10-16T11:22:22.458Z")),
        ],
    );
    p.write(NEW, &[user(NEW, "2026-10-16T11:23:00Z", "go on")]);
    let files = p.files();
    assert_eq!(forward_family(&db, &files, Some(NEW), None).unwrap(), [NEW]);
    assert_eq!(
        forward_family(&db, &files, Some(OLD), None).unwrap(),
        [OLD, NEW]
    );
    assert!(forward_family(&db, &files, None, None).unwrap().is_empty());
    assert!(
        forward_family(&db, &files, Some(""), None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_resumed_branchs_question_never_reaches_the_old_continuation() {
    // review #53 B1: OLD continued in NEW, then OLD was resumed and used on its own branch
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    p.write(
        OLD,
        &[
            continued_in(OLD, NEW, Some("2026-10-16T11:22:22Z")),
            user(OLD, "2026-10-17T09:00:00Z", "resumed"),
        ],
    );
    let files = p.files();
    // asked on the branch, after the continuation was made: NEW never saw it
    assert_eq!(
        forward_family(&db, &files, Some(OLD), Some("2026-10-17T09:05:00+00:00")).unwrap(),
        [OLD]
    );
    // asked before the continuation: it went along with the conversation
    assert_eq!(
        forward_family(&db, &files, Some(OLD), Some("2026-10-16T11:00:00+00:00")).unwrap(),
        [OLD, NEW]
    );
    // unbounded (ancestry checks only): every continuation ever made
    assert_eq!(
        forward_family(&db, &files, Some(OLD), None).unwrap(),
        [OLD, NEW]
    );
}

#[test]
fn an_undated_past_continuation_is_never_followed_when_bounded() {
    // Python test_older_conversation::test_r1_b1_an_undated_past_continuation_is_never_followed
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    p.write(
        OLD,
        &[
            continued_in(OLD, NEW, None),
            user(OLD, "2026-10-17T09:00:02Z", "on"),
        ],
    );
    let files = p.files();
    assert_eq!(
        forward_family(&db, &files, Some(OLD), Some("2026-10-17T09:00:00+00:00")).unwrap(),
        [OLD]
    );
    assert_eq!(
        forward_family(&db, &files, Some(OLD), None).unwrap(),
        [OLD, NEW]
    );
    assert_eq!(
        forward_family(&db, &files, Some(OLD), Some("")).unwrap(),
        [OLD],
        "an empty bound follows nothing past"
    );
}

#[test]
fn the_current_chain_always_counts_and_the_recorded_path_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    // OLD's transcript lives elsewhere, recorded by its SessionStart hook; undated record, still standing
    let elsewhere = dir.path().join("elsewhere.jsonl");
    std::fs::write(&elsewhere, format!("{}\n", continued_in(OLD, NEW, None))).unwrap();
    db.execute(
        "INSERT INTO sessions (session_id, transcript_path) VALUES (?, ?)",
        [OLD, elsewhere.to_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        forward_family(
            &db,
            &p.files(),
            Some(OLD),
            Some("2026-10-17T09:00:00+00:00")
        )
        .unwrap(),
        [OLD, NEW]
    );
}

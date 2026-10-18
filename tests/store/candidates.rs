use std::fs;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::store::MAX_HOOK_MISSES;
use herdr_inbox::store::candidates::has_candidates;
use rusqlite::params;

use crate::common::*;
use crate::family::Fake;

#[test]
fn an_answered_undelivered_unseen_answer_is_a_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let db = open(&path);
    let fake = Fake::default();
    let q = ask(&db, A, "Squash or merge?");
    assert!(!has_candidates(&path, A, &fake), "open: nothing to carry");
    answer(&db, q, "squash");
    assert!(has_candidates(&path, A, &fake));
    assert!(!has_candidates(&path, B, &fake), "another session's");
    for (sql, why) in [
        (
            "UPDATE questions SET delivered_at = 'x' WHERE id = ?",
            "delivered",
        ),
        (
            "UPDATE questions SET acknowledged_at = 'x' WHERE id = ?",
            "seen",
        ),
    ] {
        db.execute(sql, params![q]).unwrap();
        assert!(!has_candidates(&path, A, &fake), "{why}");
        db.execute(
            "UPDATE questions SET delivered_at = NULL, acknowledged_at = NULL WHERE id = ?",
            params![q],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO question_ext (id, hook_misses) VALUES (?, ?)",
        params![q, MAX_HOOK_MISSES - 1],
    )
    .unwrap();
    assert!(has_candidates(&path, A, &fake), "under the miss cap");
    db.execute(
        "UPDATE question_ext SET hook_misses = ? WHERE id = ?",
        params![MAX_HOOK_MISSES, q],
    )
    .unwrap();
    assert!(
        !has_candidates(&path, A, &fake),
        "at the miss cap: left to typing"
    );
    db.execute(
        "UPDATE question_ext SET hook_misses = NULL, agent_gone_at = 'x' WHERE id = ?",
        params![q],
    )
    .unwrap();
    assert!(!has_candidates(&path, A, &fake), "its agent is gone");
}

#[test]
fn a_live_predecessors_answer_is_a_candidate_through_its_real_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let db = open(&path);
    let q = ask(&db, B, "Asked before the continuation?");
    answer(&db, q, "yes");
    db.execute(
        "INSERT INTO sessions (session_id, continued_in) VALUES (?, ?)",
        [B, A],
    )
    .unwrap();
    let project = dir.path().join("projects").join("-r");
    fs::create_dir_all(&project).unwrap();
    let transcripts = Transcripts::new(Some(ProjectsRoot::new(dir.path().join("projects"))));
    assert!(
        !has_candidates(&path, A, &transcripts),
        "no transcript: the nomination alone does not count"
    );
    let record = format!(
        r#"{{"type":"continued-in","sessionId":"{B}","continuedInSessionId":"{A}","timestamp":"2026-10-17T11:00:00Z"}}"#
    );
    fs::write(project.join(format!("{B}.jsonl")), format!("{record}\n")).unwrap();
    assert!(has_candidates(&path, A, &transcripts));
    let other = "cccccccc-0000-4000-8000-00000000000c";
    let elsewhere = format!(
        r#"{{"type":"continued-in","sessionId":"{B}","continuedInSessionId":"{other}","timestamp":"2026-10-17T11:30:00Z"}}"#
    );
    fs::write(
        project.join(format!("{B}.jsonl")),
        format!("{record}\n{elsewhere}\n"),
    )
    .unwrap();
    assert!(
        !has_candidates(&path, A, &transcripts),
        "B now continues into another session, not A"
    );
    fs::write(project.join(format!("{B}.jsonl")), format!("{record}\n")).unwrap();
    assert!(has_candidates(&path, A, &transcripts));
    let resumed = format!(
        r#"{{"type":"user","sessionId":"{B}","timestamp":"2026-10-17T12:00:00Z","message":{{"content":"go on"}}}}"#
    );
    fs::write(
        project.join(format!("{B}.jsonl")),
        format!("{record}\n{resumed}\n"),
    )
    .unwrap();
    assert!(
        !has_candidates(&path, A, &transcripts),
        "B was resumed: its rows are its own again"
    );
}

#[test]
fn has_candidates_gives_up_quickly_on_a_locked_db() {
    // Python test_hook::test_has_candidates_gives_up_quickly_on_a_locked_db
    let dir = tempfile::tempdir().unwrap();
    let path = migrated(dir.path());
    let db = open(&path);
    let q = ask(&db, A, "Squash or merge?");
    answer(&db, q, "squash");
    drop(db);
    let fake = Fake::default();
    assert!(has_candidates(&path, A, &fake));
    let lock = locked(&path, "EXCLUSIVE");
    let (found, took) = timed(|| has_candidates(&path, A, &fake));
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(!found && took < LOOKUP_BUDGET, "{took:?}");
    assert!(!has_candidates(&dir.path().join("none.db"), A, &fake));
}

use herdr_inbox::store::sessions::{record_session, session_transcript};

use crate::common::*;

#[test]
fn the_transcript_path_is_recorded_and_the_rows_other_columns_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    db.execute(
        "INSERT INTO sessions (session_id, hook_seen_at, continued_in) VALUES (?, 'x', ?)",
        [A, B],
    )
    .unwrap();
    record_session(&db, A, Some("/t/a.jsonl")).unwrap();
    record_session(&db, A, None).unwrap();
    record_session(&db, A, Some("")).unwrap();
    record_session(&db, "", Some("/t/none.jsonl")).unwrap();
    assert_eq!(
        session_transcript(&db, A).unwrap().as_deref(),
        Some("/t/a.jsonl")
    );
    let kept: (String, String) = db
        .query_row(
            "SELECT hook_seen_at, continued_in FROM sessions WHERE session_id = ?",
            [A],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kept, ("x".to_owned(), B.to_owned()));
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(session_transcript(&db, B).unwrap(), None);
}

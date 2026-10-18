//! The `sessions` rows a hook reads and writes (Python `Store.record_session`,
//! `Store.session_transcript`).

use rusqlite::{Connection, OptionalExtension, params};

/// Records where `session_id`'s transcript is (the SessionStart and delivery
/// hooks' payload `transcript_path`); the row's other columns are kept.
/// Nothing when either is empty.
pub fn record_session(
    db: &Connection,
    session_id: &str,
    transcript_path: Option<&str>,
) -> rusqlite::Result<()> {
    let Some(path) = transcript_path.filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    if session_id.is_empty() {
        return Ok(());
    }
    db.execute(
        "INSERT INTO sessions (session_id, transcript_path) VALUES (?, ?) \
         ON CONFLICT(session_id) DO UPDATE SET transcript_path = excluded.transcript_path",
        params![session_id, path],
    )?;
    Ok(())
}

/// The transcript path recorded for `session_id`, if any.
pub fn session_transcript(db: &Connection, session_id: &str) -> rusqlite::Result<Option<String>> {
    if session_id.is_empty() {
        return Ok(None);
    }
    let row: Option<Option<String>> = db
        .query_row(
            "SELECT transcript_path FROM sessions WHERE session_id = ?",
            [session_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(row.flatten())
}

use rusqlite::Connection;

use super::{akey, db_id, qkey};
use crate::store::Question;
use crate::store::alerts::Alert;

// Python: hashlib.sha256(f"...".encode()).hexdigest()[:32]
#[test]
fn the_keys_are_pythons_sha256_prefixes() {
    let mut q = Question {
        created_at: "2026-01-02T03:04:05+00:00".into(),
        norm: "deploy now? é".into(),
        session_id: Some("s1".into()),
        ..Question::default()
    };
    assert_eq!(qkey("abc", &q), "81aaad2db2dff89c5c5b5a68089cf999");
    q.session_id = None; // `q.session_id or ''`
    assert_eq!(qkey("abc", &q), "d1e947f1f3446e132cda7ed499d74cc5");
    let a = Alert {
        id: 7,
        created_at: "2026-01-02T03:04:05+00:00".into(),
        kind: "agent".into(),
        severity: "info".into(),
        title: "Check 🚀".into(),
        message: None,
        pane_id: None,
        tab_id: None,
        tab_label: None,
        session_id: None,
        asked_by: None,
        herdr_server: None,
        status: "open".into(),
        acked_at: None,
        closed_at: None,
        reported_at: None,
    };
    assert_eq!(akey("abc", &a), "c789626f8afa873224338620635f2c0c");
}

#[test]
fn every_database_has_a_stable_random_id() {
    let fresh = || {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)")
            .unwrap();
        db
    };
    let db = fresh();
    let id = db_id(&db).unwrap();
    assert_eq!(id.len(), 32);
    assert!(
        id.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{id}"
    );
    assert_eq!(db_id(&db).unwrap(), id, "created once, then read");
    assert_ne!(db_id(&fresh()).unwrap(), id, "another database, another id");
    db.execute("UPDATE meta SET value = 'kept' WHERE key = 'db_id'", [])
        .unwrap();
    assert_eq!(db_id(&db).unwrap(), "kept");
}

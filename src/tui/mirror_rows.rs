//! Test rows for the remote machines' mirror, written as the puller and the
//! outbox write them (fake data only).

use serde_json::json;

use crate::store::Store;

/// The mirror's rows are stamped this.
pub const SEEN: &str = "2026-11-04T00:00:00+00:00";

/// Adds machine `id`, labelled `label`.
pub fn machine(s: &Store, id: &str, label: Option<&str>) {
    s.db()
        .execute(
            "INSERT INTO remote_machines (machine_id, label) VALUES (?, ?)",
            rusqlite::params![id, label],
        )
        .unwrap();
}

/// Mirrors the open question `id` of `machine` (database `dd`, key `k<id>`,
/// group `group`), with option `a`.
pub fn question(s: &Store, machine: &str, id: i64, group: i64) {
    let qkey = format!("k{id}");
    let data = json!({"id": id, "group": group, "qkey": qkey, "status": "open",
        "question": format!("Question Title {id}"), "options": [{"label": "a"}]});
    s.db()
        .execute(
            "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
             VALUES (?, 'dd', ?, ?, 'open', ?, ?)",
            rusqlite::params![machine, id, qkey, data.to_string(), SEEN],
        )
        .unwrap();
}

/// Mirrors the open alert `id` of `machine` (database `dd`, key `a<id>`).
pub fn alert(s: &Store, machine: &str, id: i64) {
    let akey = format!("a{id}");
    let data = json!({"id": id, "akey": akey, "kind": "blocked", "severity": "urgent",
        "status": "open", "title": "Alert Title 123"});
    s.db()
        .execute(
            "INSERT INTO remote_alerts (machine_id, db_id, alert_id, akey, status, data_json, first_seen_at) \
             VALUES (?, 'dd', ?, ?, 'open', ?, ?)",
            rusqlite::params![machine, id, akey, data.to_string(), SEEN],
        )
        .unwrap();
}

/// Adds outbox row `id` on question 1 of `machine`, `state`, expiring at
/// `expires`.
pub fn outbox(s: &Store, id: &str, machine: &str, state: &str, expires: &str) {
    s.db()
        .execute(
            "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at, \
             expires_at, state) VALUES (?, ?, 'dd', 1, 'k1', 'answer', ?, ?, ?)",
            rusqlite::params![id, machine, SEEN, expires, state],
        )
        .unwrap();
}

/// The outbox as (action, remote id, qkey, db id), oldest first.
pub fn queued(s: &Store) -> Vec<(String, i64, String, String)> {
    let mut stmt = s
        .db()
        .prepare("SELECT action, remote_id, qkey, db_id FROM remote_outbox ORDER BY rowid")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// An outbox row's state and result.
pub fn state(s: &Store, id: &str) -> (String, Option<String>) {
    s.db()
        .query_row(
            "SELECT state, result FROM remote_outbox WHERE request_id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

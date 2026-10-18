//! The resume-marker guard (#53) at the watcher: herdr reports OLD for the pane after OLD
//! continued in A; whether the pane runs OLD or A depends on where OLD was resumed.

use std::fs;
use std::time::SystemTime;

use herdr_inbox::hook::payload::Payload;
use herdr_inbox::rewake::run;
use rusqlite::params;

use crate::common::{A, answer, ask};
use crate::world::{FakeClock, World};

const OLD: &str = "01d00000-0000-4000-8000-000000000001";

/// OLD's transcript continued in A (10:00); OLD was resumed afterwards (now) in `marker_pane`;
/// herdr reports OLD for w1:p1. One answer pending for `asker`.
fn world(marker_pane: &str, asker: &str) -> (World, i64) {
    let w = World::new();
    let record = serde_json::json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": A,
                                    "timestamp": "2026-10-17T10:00:00Z"});
    fs::write(
        w.dir
            .path()
            .join("projects/-r")
            .join(format!("{OLD}.jsonl")),
        format!("{record}\n"),
    )
    .unwrap();
    let starts = w.dir.path().join("starts");
    fs::create_dir_all(&starts).unwrap();
    fs::write(starts.join(OLD), marker_pane).unwrap();
    fs::File::options()
        .write(true)
        .open(starts.join(OLD))
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();
    w.agent(Some(OLD), "idle");
    let db = w.conn();
    let id = ask(&db, asker, "Which colour?");
    answer(&db, id, "amber");
    db.execute(
        "UPDATE questions SET created_at = ? WHERE id = ?",
        params![herdr_inbox::clock::now(), id],
    )
    .unwrap();
    (w, id)
}

fn watch(w: &World, session: &str) -> Option<String> {
    let payload = Payload {
        session_id: Some(session.into()),
        ..Payload::default()
    };
    run(
        &w.watcher(
            &FakeClock::new(|n, t| {
                if n == 1 {
                    t.set(1e9)
                }
            }),
            Some("w1:p1"),
        ),
        &payload,
    )
}

#[test]
fn resumed_in_this_pane_old_runs_here() {
    let (w, id) = world("w1:p1", OLD);
    assert_eq!(
        watch(&w, A),
        None,
        "the pane runs OLD, not A: A's watch ends"
    );
    assert!(
        watch(&w, OLD).is_some_and(|t| t.contains(&format!("Q{id} "))),
        "OLD's watcher wakes OLD"
    );
}

#[test]
fn resumed_in_another_pane_this_pane_runs_the_continuation() {
    let (w, id) = world("w1:p9", A);
    assert_eq!(
        watch(&w, OLD),
        None,
        "OLD runs in p9, not here: OLD's watch in p1 ends"
    );
    assert!(
        watch(&w, A).is_some_and(|t| t.contains(&format!("Q{id} "))),
        "A's watcher wakes A"
    );
}

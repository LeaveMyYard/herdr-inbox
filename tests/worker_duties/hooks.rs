//! Hook deliveries the worker settles without a landing of the whole answer
//! (Python `service.verify_hook_deliveries`). Synthetic data.

use std::time::Duration;

use herdr_inbox::clock::{now, now_minus};
use herdr_inbox::worker::duties::Round;
use herdr_inbox::worker::duties::hooks::verify_hook_deliveries;
use rusqlite::params;
use serde_json::{Value, json};

use crate::World;
use crate::failed::{NEW, OLD};

fn late() -> String {
    now_minus(Duration::from_secs(60))
}

#[test]
fn a_hook_reference_that_landed_alone_is_verified_but_never_seen() {
    let mut w = World::new();
    let qid = w.hook_sent(&now());
    let reference = w.reference_text(qid);
    w.write(crate::A, &[json!({"type": "attachment", "timestamp": now(),
        "attachment": {"type": "hook_additional_context", "content": [reference], "hookEvent": "PostToolBatch"}})]);
    w.verify_hooks();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.delivered_at.is_some() && q.verified_at.is_some(), "{q:?}");
    assert!(q.acknowledged_at.is_none(), "{q:?}");
}

#[test]
fn a_hook_delivery_with_no_transcript_stays_sent_and_is_never_seen() {
    let mut w = World::new();
    let qid = w.hook_sent(&late());
    std::fs::remove_file(w.transcript()).unwrap();
    w.verify_hooks();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(
        q.delivered_at.is_some() && q.acknowledged_at.is_none(),
        "{q:?}"
    );
    assert_eq!(w.store.hook_misses(qid).unwrap(), 0);
}

#[test]
fn an_unparseable_send_time_counts_as_time_up() {
    let mut w = World::new();
    let qid = w.hook_sent("x");
    w.verify_hooks();
    assert_eq!(w.store.hook_misses(qid).unwrap(), 1);
}

fn hook_context(text: String) -> Value {
    json!({"type": "attachment", "timestamp": now(),
        "attachment": {"type": "hook_additional_context", "content": [text], "hookEvent": "PostToolBatch"}})
}

#[test]
fn a_store_error_during_the_lookup_aborts_the_pass_and_settles_nothing() {
    let mut w = World::new();
    let first = w.hook_sent(&late());
    let second = w.hook_sent(&late());
    // both are past their time: read as "not found", the failed lookup would count a miss
    w.sql("ALTER TABLE sessions RENAME TO sessions_lost", []);
    let ran = w.with_round(|round| {
        verify_hook_deliveries(&mut Round {
            ctx: &mut *round.ctx,
            server: round.server,
        })
    });
    let err = ran.unwrap_err().to_string();
    assert!(err.contains("sessions"), "{err}");
    for qid in [first, second] {
        assert_eq!(w.store.hook_misses(qid).unwrap(), 0);
        let q = w.store.get(qid).unwrap().unwrap();
        assert!(q.delivered_at.is_some() && q.verified_at.is_none(), "{q:?}");
    }
    let sent: i64 = w
        .store
        .db()
        .query_row(
            "SELECT COUNT(*) FROM question_ext WHERE hook_sent_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sent, 2, "both stay hook-sent, for the next pass");
}

#[test]
fn a_hook_answer_found_in_a_forward_continuation_is_verified() {
    let mut w = World::new();
    let qid = w.filed("w1:p3", OLD, "Question Text 201", "Answer Text 201");
    w.sql(
        "UPDATE questions SET created_at = '2026-01-01T00:00:00+00:00' WHERE id = ?",
        params![qid],
    );
    w.mark_hook_sent(qid, &now());
    w.continued_then_resumed(None); // OLD continued in NEW after it asked
    let reference = w.reference_text(qid);
    w.write(NEW, &[hook_context(reference)]);
    w.verify_hooks();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.verified_at.is_some(), "{q:?}");
}

#[test]
fn a_hook_answer_found_only_in_an_older_conversation_is_never_verified() {
    let mut w = World::new();
    let qid = w.filed("w1:p3", NEW, "Question Text 202", "Answer Text 202");
    w.mark_hook_sent(qid, &late());
    w.continued_then_resumed(None); // NEW continues OLD: OLD is its ancestor, not its family
    let reference = w.reference_text(qid);
    w.write(OLD, &[hook_context(reference)]);
    w.verify_hooks();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.verified_at.is_none(), "{q:?}");
    assert_eq!(w.store.hook_misses(qid).unwrap(), 1);
}

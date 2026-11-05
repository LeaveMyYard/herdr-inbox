//! Hook deliveries the worker settles without a landing of the whole answer
//! (Python `service.verify_hook_deliveries`). Synthetic data.

use std::time::Duration;

use herdr_inbox::clock::{now, now_minus};
use serde_json::json;

use crate::World;

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

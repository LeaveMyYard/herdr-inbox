//! Exactly-once typing under a claim (Python tests/test_background_keys.py):
//! a claim held by another process keeps every other sender out, and only a
//! claim left behind by a dead one is taken over.

use std::time::Duration;

use herdr_inbox::delivery::run::deliver_batch;
use herdr_inbox::store::Store;
use inbox_core::deliver::Mode;

use crate::World;

impl World {
    /// `qid` delivered as an answer is (Python `service.deliver`).
    fn deliver(&mut self, qid: i64) {
        let q = self.store.get(qid).unwrap().unwrap();
        self.with_ctx(|ctx, _| deliver_batch(ctx, &[q], Mode::default(), None).unwrap());
    }
}

#[test]
fn delivery_claims_the_row_before_sending() {
    let mut w = World::new();
    let qid = w.ask();
    w.fake.set(|k| k.fail_prompt = true);
    w.answer(qid, "idle");
    w.fake.set(|k| k.fail_prompt = false);
    assert_eq!(w.store.get(qid).unwrap().unwrap().delivered_at, None);
    let other = Store::open(&w.db, Duration::from_secs(5)).unwrap();
    assert!(other.claim_delivery(qid).unwrap().is_some()); // e.g. another sender got there first
    w.fake.reset();

    w.tick(); // the worker's retry
    w.deliver(qid);
    assert!(w.fake.prompts().is_empty());
    assert!(other.claim_delivery(qid).unwrap().is_none());

    // a claim left behind by a process that died is taken over after a while
    w.sql(
        "UPDATE questions SET delivery_error = 'sending:2020-01-01T00:00:00+00:00' WHERE id = ?",
        [qid],
    );
    w.deliver(qid);
    assert!(w.store.get(qid).unwrap().unwrap().delivered_at.is_some());
    assert!(w.store.failed_deliveries().unwrap().is_empty());
}

//! Unverified acknowledgements re-checked (Python `Store._reconcile_unverified`
//! through the worker: tests/test_delivery_worker.py's old-server rows and
//! tests/test_verified_delivery.py's re-check rows). Python re-checks on
//! every database open with maintenance; the worker opens each tick, so
//! "every open" is a [`World::full_tick`] (or `reconcile_unverified`) here.

use std::time::Duration;

use herdr_inbox::clock::now_minus;
use herdr_inbox::store::outcomes::unverified::{RECONCILE_BATCH, RESET_KEY};
use herdr_inbox::worker::duties::unverified::reconcile_unverified;
use rusqlite::params;
use serde_json::json;

use crate::World;

impl World {
    /// Answered without delivery.
    fn answered(&mut self) -> i64 {
        let qid = self.ask();
        self.store.set_answer(qid, "yes", None).unwrap();
        qid
    }

    /// A pre-0.5.1 server's "typed = seen" at `at` (Python `_signature` /
    /// `_old_server_delivery`).
    fn signature(&self, qid: i64, at: &str) {
        self.sql(
            "UPDATE questions SET delivered_at = ?, acknowledged_at = ? WHERE id = ?",
            params![at, at, qid],
        );
    }

    fn q(&self, qid: i64) -> herdr_inbox::store::Question {
        self.store.get(qid).unwrap().unwrap()
    }

    fn landed(&self, qid: i64) {
        let text = self.batch_text(&[qid]);
        self.write(
            crate::A,
            &[json!({"type": "user", "message": {"content": text}})],
        );
    }
}

fn ago(secs: u64) -> String {
    now_minus(Duration::from_secs(secs))
}

#[test]
fn an_old_servers_unverified_ack_is_requeued_and_delivered() {
    let mut w = World::new();
    let qid = w.answered();
    w.signature(qid, &ago(60)); // settled: older than the verify window
    w.fake.set(|k| k.status = "idle".into());
    w.full_tick();
    assert!(w.q(qid).verified_at.is_some() && w.fake.prompts().len() == 1);
}

#[test]
fn old_requeue_window_does_not_retype_old_answers() {
    let mut w = World::new();
    let qid = w.answered();
    // inside the one-off 7-day sweep, outside the 1 h requeue and 24 h signature windows
    let old = ago(2 * 86_400);
    w.sql(
        "UPDATE questions SET delivered_at = ?, acknowledged_at = ?, answered_at = ? WHERE id = ?",
        params![old, old, old, qid],
    );
    w.sql("DELETE FROM meta WHERE key = ?", params![RESET_KEY]);
    w.fake.set(|k| k.status = "idle".into());
    w.full_tick();
    assert!(w.fake.prompts().is_empty() && w.q(qid).acknowledged_at.is_none()); // un-seen only
}

#[test]
fn h1_a_just_typed_old_server_delivery_is_not_requeued_before_it_settles() {
    let mut w = World::new();
    let qid = w.answered();
    w.signature(qid, &ago(0));
    w.fake.set(|k| k.status = "idle".into());
    w.full_tick(); // not settled: left alone
    assert!(w.fake.prompts().is_empty() && w.q(qid).acknowledged_at.is_some());
    w.landed(qid); // ...the original lands
    w.signature(qid, &ago(60));
    w.full_tick();
    assert!(w.fake.prompts().is_empty() && w.q(qid).verified_at.is_some());
}

#[test]
fn h1_no_transcript_means_un_see_only_never_retype() {
    let mut w = World::new();
    let qid = w.answered();
    w.signature(qid, &ago(60));
    std::fs::remove_file(w.transcript()).unwrap();
    w.fake.set(|k| {
        k.status = "idle".into();
        k.land = "none";
    });
    w.full_tick();
    let q = w.q(qid);
    assert!(w.fake.prompts().is_empty() && q.acknowledged_at.is_none() && q.delivered_at.is_some());
}

#[test]
fn migration_uses_the_verifier() {
    let mut w = World::new();
    let (landed, lost) = (w.answered(), w.answered());
    for qid in [landed, lost] {
        w.signature(qid, &ago(3 * 86_400));
    }
    w.landed(landed);
    w.sql("DELETE FROM meta WHERE key = ?", params![RESET_KEY]); // as if this DB predates 0.5.1
    reconcile_unverified(&w.store, &w.files).unwrap();
    assert!(w.q(landed).acknowledged_at.is_some() && w.q(landed).verified_at.is_some());
    assert!(w.q(lost).acknowledged_at.is_none());
}

#[test]
fn old_server_deliveries_are_rechecked_on_every_open_and_cannot_loop() {
    let mut w = World::new();
    let qid = w.answered();
    for round in 1..4 {
        w.signature(qid, &ago(60)); // an old server re-acks it
        w.fake.set(|k| k.status = "working".into()); // nobody types meanwhile
        reconcile_unverified(&w.store, &w.files).unwrap();
        let expected_seen = round > 2; // MAX_RESETS: the third re-ack by an old server stays
        assert_eq!(
            w.q(qid).acknowledged_at.is_some(),
            expected_seen,
            "round {round}"
        );
    }
    assert!(w.fake.prompts().is_empty()); // never re-typed by the reset itself
}

#[test]
fn check_answers_acknowledgement_is_not_the_old_server_signature() {
    let mut w = World::new();
    let qid = w.answered();
    w.sql(
        "UPDATE questions SET delivered_at = '2026-10-14T00:00:00+00:00' WHERE id = ?",
        params![qid],
    );
    w.store.acknowledge(&[qid]).unwrap(); // the agent fetched it later
    w.sql(
        "INSERT OR IGNORE INTO meta (key, value) VALUES (?, 'x')",
        params![RESET_KEY],
    );
    reconcile_unverified(&w.store, &w.files).unwrap();
    assert!(w.q(qid).acknowledged_at.is_some());
}

#[test]
fn seven_day_sweep_continues_past_one_batch() {
    let mut w = World::new();
    let n = RECONCILE_BATCH as usize * 2 + 5;
    let ids: Vec<i64> = (0..n).map(|_| w.answered()).collect();
    let old = ago(3 * 86_400);
    w.sql(
        "UPDATE questions SET delivered_at = ?, acknowledged_at = ?",
        params![old, old],
    );
    w.sql("DELETE FROM meta WHERE key = ?", params![RESET_KEY]);
    for _ in 0..2 {
        reconcile_unverified(&w.store, &w.files).unwrap();
        let done: Option<i64> = w
            .store
            .db()
            .query_row("SELECT 1 FROM meta WHERE key = ?", [RESET_KEY], |r| {
                r.get(0)
            })
            .ok();
        assert!(done.is_none()); // full batches: the sweep is not done yet
    }
    reconcile_unverified(&w.store, &w.files).unwrap();
    assert!(ids.iter().all(|q| w.q(*q).acknowledged_at.is_none()));
    let done: Option<i64> = w
        .store
        .db()
        .query_row("SELECT 1 FROM meta WHERE key = ?", [RESET_KEY], |r| {
            r.get(0)
        })
        .ok();
    assert_eq!(done, Some(1));
}

#[test]
fn an_old_servers_ack_that_landed_only_in_an_older_session_is_never_verified() {
    use crate::failed::{NEW, OLD};
    let mut w = World::new();
    let qid = w.filed("w1:p3", NEW, "Landed in the ancestor only?", "yes");
    w.signature(qid, &ago(60));
    w.write(
        OLD,
        &[json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW})],
    );
    w.record_session(OLD);
    let text = w.batch_text(&[qid]);
    w.write(
        OLD,
        &[json!({"type": "user", "message": {"content": text}})],
    );
    w.write(NEW, &[json!({"type": "ai-title", "aiTitle": "x"})]);
    w.record_session(NEW);
    reconcile_unverified(&w.store, &w.files).unwrap();
    let q = w.q(qid);
    assert!(q.verified_at.is_none() && q.acknowledged_at.is_none()); // re-queued, never verified
}

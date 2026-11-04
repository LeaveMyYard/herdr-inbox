//! Answers whose agent is gone (Python tests/test_agent_gone.py, the
//! worker's half): marked once, notified once, never polled again; kept for
//! `check_answers`.

use inbox_core::deliver::MAX_STALE_RETRIES;
use rusqlite::params;

use crate::World;

impl World {
    /// Python `gone_answer`: asked, then answered once the pane has no agent.
    fn gone_answer(&mut self) -> i64 {
        let qid = self.ask();
        self.fake.set(|k| k.agent = false);
        self.answer(qid, "idle");
        qid
    }
}

#[test]
fn worker_marks_a_gone_agent_once_notifies_once_then_stops_polling() {
    let mut w = World::new();
    let qid = w.gone_answer();
    for _ in 0..MAX_STALE_RETRIES + 3 {
        w.tick();
    }
    assert!(w.store.agent_gone(qid).unwrap());
    w.fake.reset();
    for _ in 0..5 {
        w.tick();
    }
    assert_eq!(w.fake.agent_gets(), 0); // no more polling
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.acknowledged_at.is_none() && q.status == "answered"); // kept for check_answers
}

#[test]
fn gone_notification_says_the_answer_is_kept() {
    let mut w = World::new();
    let qid = w.gone_answer();
    for _ in 0..MAX_STALE_RETRIES + 3 {
        w.tick();
    }
    let shows = w.shown("gone");
    assert_eq!(shows.len(), 1);
    assert!(shows[0].0.contains(&format!("Q{qid}")) && shows[0].1.contains("check_answers"));
    // the pane is named as asked: a gone agent's tab is not looked up
    assert_eq!(shows[0].0, format!("Q{qid}: the agent in tab-A is gone"));
}

#[test]
fn hidden_gone_rows_are_not_polled() {
    let mut w = World::new();
    let qid = w.gone_answer();
    w.store.hide_failed(qid).unwrap();
    w.store
        .mark_agent_gone(qid, "no live agent in pane w1:p1")
        .unwrap();
    w.fake.reset();
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.fake.agent_gets(), 0);
}

#[test]
fn legacy_rows_of_a_gone_pane_are_not_polled_every_tick() {
    let mut w = World::new();
    let qid = w.ask();
    w.sql(
        "UPDATE question_ext SET herdr_server = NULL WHERE id = ?",
        params![qid],
    );
    w.fake.set(|k| k.agent = false);
    w.answer(qid, "idle");
    w.fake.reset();
    for _ in 0..20 {
        w.tick();
    }
    assert!(w.fake.agent_gets() <= 1); // backed off, not once per tick
}

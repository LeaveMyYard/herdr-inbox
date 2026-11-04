//! Conversations that moved, continued, or took their answer as hook
//! context (Python tests/test_pane_moved.py, test_session_continuation.py
//! and test_hook_delivery.py, the worker's halves).

use herdr_inbox::clock::{now, now_minus};
use rusqlite::params;
use serde_json::{Value, json};

use crate::World;
use crate::failed::{NEW, OLD};

fn agents(pairs: &[(&str, &str)]) -> Value {
    pairs
        .iter()
        .map(|(p, s)| {
            json!({"pane_id": p, "tab_id": "w1:t9", "agent": "claude", "agent_status": "idle",
                             "agent_session": {"value": s}})
        })
        .collect()
}

impl World {
    /// Python `gone`: answered in w1:p1, then marked gone there.
    fn gone_in_p1(&mut self) -> i64 {
        let qid = self.ask();
        self.store.set_answer(qid, "here", None).unwrap();
        self.sql(
            "UPDATE questions SET pane_id = 'w1:p1', tab_label = 'old-tab', \
             delivery_error = 'agent gone: no live agent in pane w1:p1' WHERE id = ?",
            params![qid],
        );
        self.store
            .mark_agent_gone(qid, "no live agent in pane w1:p1")
            .unwrap();
        qid
    }

    /// The conversation now runs in w1:p7 (and another one in w1:p8).
    fn moved_to_p7(&self) {
        self.fake.set(|k| {
            k.agents = agents(&[
                ("w1:p7", crate::A),
                ("w1:p8", "bbbbbbbb-0000-4000-8000-00000000000b"),
            ])
        });
    }

    fn list_calls(&self) -> usize {
        self.fake.stub.params_of("agent.list").len()
    }
}

#[test]
fn an_answer_already_marked_gone_is_revived_when_its_agent_turns_up() {
    let mut w = World::new();
    let qid = w.gone_in_p1();
    w.moved_to_p7();
    w.full_tick();
    w.full_tick();
    let q = w.store.get(qid).unwrap().unwrap();
    assert_eq!(q.pane_id.as_deref(), Some("w1:p7"));
    assert!(!w.store.agent_gone(qid).unwrap() && q.delivered_at.is_some());
    assert_eq!(q.tab_label.as_deref(), Some("my-tab")); // the new tab's label
    assert!(w.fake.prompts().iter().all(|p| p == "w1:p7"));
}

#[test]
fn another_herdr_servers_gone_rows_are_left_alone() {
    let mut w = World::new();
    let qid = w.gone_in_p1();
    w.sql(
        "UPDATE question_ext SET herdr_server = '/other.sock' WHERE id = ?",
        params![qid],
    );
    w.moved_to_p7();
    w.full_tick();
    assert!(w.store.agent_gone(qid).unwrap());
    assert_eq!(
        w.store.get(qid).unwrap().unwrap().pane_id.as_deref(),
        Some("w1:p1")
    );
}

#[test]
fn a_hidden_answer_is_never_revived() {
    let mut w = World::new();
    let qid = w.gone_in_p1();
    w.store.hide_failed(qid).unwrap();
    w.moved_to_p7();
    w.full_tick();
    w.full_tick();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(
        q.pane_id.as_deref() == Some("w1:p1")
            && w.store.agent_gone(qid).unwrap()
            && q.delivered_at.is_none()
    );
    assert!(w.fake.prompts().is_empty());
}

#[test]
fn a_send_in_flight_is_not_revived_under_it() {
    let mut w = World::new();
    let qid = w.gone_in_p1();
    w.sql(
        "UPDATE questions SET delivery_error = ? WHERE id = ?",
        params![format!("sending:{}:x", now()), qid],
    );
    w.moved_to_p7();
    w.full_tick();
    assert_eq!(
        w.store.get(qid).unwrap().unwrap().pane_id.as_deref(),
        Some("w1:p1")
    );
}

#[test]
fn revival_backs_off_per_answer() {
    let mut w = World::new();
    w.fake
        .set(|k| k.agents = agents(&[("w1:p8", "bbbbbbbb-0000-4000-8000-00000000000b")])); // really gone
    w.gone_in_p1();
    w.fake.reset();
    w.full_tick();
    w.full_tick();
    assert_eq!(w.list_calls(), 1); // the second tick is within REVIVE_EVERY
    w.memory.revive_checked.clear();
    w.full_tick();
    assert_eq!(w.list_calls(), 2);
}

#[test]
fn the_worker_records_the_continuation_of_a_session_in_play() {
    let mut w = World::new();
    let q = w.filed("w9:p9", OLD, "Open one?", "x");
    w.sql(
        "UPDATE questions SET status = 'open', answer = NULL WHERE id = ?",
        params![q],
    );
    w.write(
        OLD,
        &[json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW})],
    );
    w.full_tick();
    let next: Option<String> = w
        .store
        .db()
        .query_row(
            "SELECT continued_in FROM sessions WHERE session_id = ?",
            [OLD],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(next.as_deref(), Some(NEW));
    assert_eq!(
        w.store.get(q).unwrap().unwrap().session_id.as_deref(),
        Some(OLD)
    ); // never rewritten
}

impl World {
    /// The hook sent `qid`'s answer at `sent_at` (as `record_hook_delivery` leaves it).
    fn hook_sent(&mut self, sent_at: &str) -> i64 {
        let qid = self.ask();
        self.store.set_answer(qid, "squash", None).unwrap();
        self.sql(
            "UPDATE questions SET delivered_at = ?, delivery_error = NULL WHERE id = ?",
            params![sent_at, qid],
        );
        self.sql(
            "UPDATE question_ext SET hook_sent_at = ? WHERE id = ?",
            params![sent_at, qid],
        );
        qid
    }
}

#[test]
fn the_worker_verifies_a_hook_delivery() {
    let mut w = World::new();
    let qid = w.hook_sent(&now());
    let text = w.batch_text(&[qid]);
    w.write(crate::A, &[json!({"type": "attachment", "timestamp": now(),
        "attachment": {"type": "hook_additional_context", "content": [text], "hookEvent": "PostToolBatch"}})]);
    w.full_tick();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.acknowledged_at.is_some() && q.verified_at.is_some());
}

#[test]
fn an_unseen_hook_delivery_goes_back_to_pending_with_a_miss() {
    let mut w = World::new();
    let fresh = w.hook_sent(&now());
    let late = w.hook_sent(&now_minus(std::time::Duration::from_secs(60)));
    w.fake.set(|k| k.status = "working".into()); // nobody types it meanwhile
    w.full_tick();
    assert!(w.store.get(fresh).unwrap().unwrap().delivered_at.is_some()); // its time is not up
    let q = w.store.get(late).unwrap().unwrap();
    assert!(q.delivered_at.is_none() && q.acknowledged_at.is_none());
    // back to pending (the same tick's retry then finds the agent busy)
    assert!(
        q.delivery_error
            .as_deref()
            .is_some_and(|e| e.starts_with("pending: "))
    );
    assert_eq!(w.store.hook_misses(late).unwrap(), 1);
}

#[test]
fn an_unseen_hook_delivery_alone_is_pending_hook_unseen() {
    let mut w = World::new();
    let late = w.hook_sent(&now_minus(std::time::Duration::from_secs(60)));
    w.verify_hooks();
    let q = w.store.get(late).unwrap().unwrap();
    assert_eq!(
        q.delivery_error,
        Some(format!("pending: {}", inbox_core::verify::HOOK_UNSEEN))
    );
}

#[test]
fn hook_verification_is_scoped_to_the_workers_herdr_server() {
    let mut w = World::new();
    let qid = w.hook_sent(&now_minus(std::time::Duration::from_secs(60)));
    w.sql(
        "UPDATE question_ext SET herdr_server = '/other.sock' WHERE id = ?",
        params![qid],
    );
    w.full_tick();
    assert_eq!(w.store.hook_misses(qid).unwrap(), 0);
    assert!(w.store.get(qid).unwrap().unwrap().delivered_at.is_some());
}

#[test]
fn a_dismissed_question_is_never_revived() {
    let mut w = World::new();
    let qid = w.ask();
    w.sql(
        "UPDATE questions SET pane_id = 'w1:p1' WHERE id = ?",
        params![qid],
    );
    w.store.mark_agent_gone(qid, "x").unwrap();
    w.sql(
        "UPDATE questions SET status = 'dismissed' WHERE id = ?",
        params![qid],
    );
    w.moved_to_p7();
    w.full_tick();
    assert_eq!(
        w.store.get(qid).unwrap().unwrap().pane_id.as_deref(),
        Some("w1:p1")
    );
}

#[test]
fn an_open_alert_alone_keeps_its_sessions_continuation_recorded() {
    let mut w = World::new();
    w.sql(
        "INSERT INTO alerts (created_at, kind, severity, title, session_id, status) \
         VALUES (?, 'agent', 'warning', 'Check it', ?, 'open')",
        params![now(), OLD],
    );
    w.write(
        OLD,
        &[json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW})],
    );
    w.full_tick();
    let next: Option<String> = w
        .store
        .db()
        .query_row(
            "SELECT continued_in FROM sessions WHERE session_id = ?",
            [OLD],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(next.as_deref(), Some(NEW));
}

//! The retry loop (Python tests/test_delivery_worker.py's `worker.tick`
//! scenarios). Python sets `BUSY_GRACE` to 0 where an answer must be typed
//! into a busy agent at once; here its `pending_since` is moved back
//! instead ([`World::past_grace`]).

use rusqlite::params;
use serde_json::json;

use crate::{PANE_B, World};

impl World {
    /// The busy grace of `qid` is over (Python `BUSY_GRACE = 0`).
    fn past_grace(&self, qid: i64) {
        self.sql(
            "UPDATE question_ext SET pending_since = '2026-01-01T00:00:00+00:00' WHERE id = ?",
            params![qid],
        );
    }

    fn verified(&self, qid: i64) -> bool {
        self.store.get(qid).unwrap().unwrap().verified_at.is_some()
    }

    fn error(&self, qid: i64) -> String {
        self.store
            .get(qid)
            .unwrap()
            .unwrap()
            .delivery_error
            .unwrap_or_default()
    }
}

#[test]
fn one_tick_delivers_pending_answers_for_every_pane() {
    let mut w = World::new();
    let a = w.pending_answer("blocked");
    let server = w.server.clone();
    let b = w.ask_on(PANE_B, Some(&server));
    w.answer(b, "blocked");
    w.fake.set(|k| k.status = "idle".into());
    w.tick();
    assert!(w.verified(a) && w.verified(b));
    let mut targets = w.fake.prompts();
    targets.sort();
    assert_eq!(targets, ["w1:p1", "w1:p2"]); // one message per pane
}

#[test]
fn a_busy_miss_waits_for_idle_and_shows_as_not_delivered() {
    let mut w = World::new();
    w.fake.set(|k| k.land = "none");
    let qid = w.pending_answer("working");
    w.past_grace(qid);
    w.tick();
    w.tick();
    assert_eq!(w.fake.prompts().len(), 1); // one busy try, then wait for idle
    assert!(
        w.error(qid)
            .starts_with("pending: busy: typed while the agent was busy")
    );
    w.fake.set(|k| {
        k.status = "done".into();
        k.land = "user";
    });
    w.tick();
    assert!(w.verified(qid) && w.fake.prompts().len() == 2);
}

const PANEL_FOCUSED: &str = "✻ Waiting for 3 background agents to finish\n\
──────────────────────────────────────────────\n❯\n\
──────────────────────────────────────────────\n  ↑/↓ to select · ctrl+x ctrl+k to stop all agents\n\
❯ ● main\n  ◯ general-purpose Compute (1)";

#[test]
fn a_focused_task_panel_is_not_typed_into() {
    let mut w = World::new();
    w.fake.set(|k| k.screen = PANEL_FOCUSED.into());
    let qid = w.pending_answer("working");
    w.past_grace(qid);
    w.tick();
    assert!(w.fake.prompts().is_empty());
    assert!(w.error(qid).contains("panel"));
    w.fake.set(|k| k.screen = String::new());
    w.tick();
    assert!(w.verified(qid));
}

#[test]
fn stuck_answers_raise_one_notification() {
    let mut w = World::new();
    let a = w.pending_answer("working"); // busy all along (within its grace): never typed
    let b = w.pending_answer("working");
    w.sql(
        "UPDATE questions SET answered_at = '2026-10-15T00:00:00+00:00' WHERE id IN (?, ?)",
        params![a, b],
    );
    w.tick();
    w.tick();
    let shows = w.shown("not delivered");
    assert_eq!(shows.len(), 1);
    assert_eq!(shows[0].0, format!("Q{a}–Q{b} not delivered to my-tab"));
    assert!(shows[0].1.contains("check_answers"));
}

#[test]
fn unverifiable_sends_are_not_reported_as_not_delivered() {
    let mut w = World::new();
    std::fs::remove_file(w.transcript()).unwrap(); // no transcript: sent, unseen by design
    w.fake.set(|k| k.land = "none");
    let qid = w.pending_answer("idle");
    w.sql(
        "UPDATE questions SET answered_at = '2026-10-15T00:00:00+00:00' WHERE id = ?",
        params![qid],
    );
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.delivered_at.is_some() && q.verified_at.is_none() && q.acknowledged_at.is_none());
    w.tick();
    assert!(w.shown("not delivered").is_empty());
}

#[test]
fn an_agent_at_a_prompt_gets_a_waiting_notification() {
    let mut w = World::new();
    let qid = w.pending_answer("blocked");
    w.sql(
        "UPDATE questions SET answered_at = '2026-10-15T00:00:00+00:00' WHERE id = ?",
        params![qid],
    );
    w.tick();
    let shows: Vec<_> = w
        .notes
        .iter()
        .filter(|(t, _)| t.starts_with("waiting:"))
        .collect();
    assert_eq!(shows.len(), 1);
    assert!(shows[0].0.contains("is at a prompt") && !shows[0].1.contains("check_answers"));
}

#[test]
fn h1_a_retry_checks_the_transcript_before_typing_again() {
    let mut w = World::new();
    w.fake.set(|k| k.land = "none");
    let qid = w.pending_answer("idle");
    assert!(w.error(qid).starts_with("not verified:") && w.fake.prompts().len() == 1);
    // the late record: the real message (the check matches the full text)
    let q = w.store.get(qid).unwrap().unwrap();
    let text = format!("[inbox] Q{qid} answered — {}: yes", q.question);
    let line = json!({"type": "user", "message": {"content": text}});
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(w.transcript())
        .unwrap();
    std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes()).unwrap();
    w.tick();
    assert!(w.fake.prompts().len() == 1 && w.verified(qid));
}

#[test]
fn m1_a_worker_leaves_other_servers_rows_alone() {
    let mut w = World::new();
    let qid = w.ask_on(("w1:p1", "w1:t1"), Some("/tmp/other.sock"));
    w.answer(qid, "blocked");
    let attempts = w.store.get(qid).unwrap().unwrap().delivery_attempts;
    w.fake.set(|k| k.status = "idle".into());
    w.fake.reset();
    for _ in 0..3 {
        w.tick();
    }
    assert!(w.fake.prompts().is_empty());
    assert_eq!(
        w.store.get(qid).unwrap().unwrap().delivery_attempts,
        attempts
    );
    assert_eq!(w.fake.agent_gets(), 0); // not even looked up on the wrong server
}

#[test]
fn m1_legacy_rows_need_a_matching_session() {
    let mut w = World::new();
    let qid = w.pending_answer("blocked");
    w.sql(
        "UPDATE question_ext SET herdr_server = NULL WHERE id = ?",
        params![qid],
    );
    w.fake.set(|k| {
        k.status = "idle".into();
        k.session = Some("someone-else".into()); // that pane id, another conversation
    });
    let typed = w.fake.prompts().len();
    w.tick();
    assert_eq!(w.fake.prompts().len(), typed);
    assert_eq!(w.store.get(qid).unwrap().unwrap().delivery_attempts, 0); // no caps burnt
    w.fake.set(|k| k.session = Some(crate::A.into()));
    w.sql(
        "UPDATE question_ext SET legacy_checked_at = NULL WHERE id = ?",
        params![qid],
    );
    w.tick();
    assert!(w.verified(qid));
    let qid2 = w.pending_answer("blocked");
    w.sql(
        "UPDATE question_ext SET herdr_server = NULL WHERE id = ?",
        params![qid2],
    );
    w.sql(
        "UPDATE questions SET session_id = NULL WHERE id = ?",
        params![qid2],
    );
    w.fake.set(|k| k.status = "idle".into());
    let n = w.fake.prompts().len();
    w.tick();
    assert_eq!(w.fake.prompts().len(), n); // never a pane-id-only match
}

#[test]
fn other_servers_rows_do_not_keep_a_worker_alive() {
    let mut w = World::new();
    let qid = w.ask_on(("w1:p1", "w1:t1"), Some("/tmp/other.sock"));
    w.answer(qid, "blocked"); // pending, but on another herdr server
    assert_eq!(w.tick(), 0);
}

#[test]
fn unmatched_legacy_rows_do_not_keep_a_worker_alive() {
    let mut w = World::new();
    let qid = w.pending_answer("blocked");
    w.sql(
        "UPDATE question_ext SET herdr_server = NULL WHERE id = ?",
        params![qid],
    );
    w.fake.set(|k| k.session = Some("someone-else".into()));
    assert_eq!(w.tick(), 0);
}

#[test]
fn own_pending_rows_still_count() {
    let mut w = World::new();
    w.pending_answer("blocked"); // blocked agent: stays pending, on this server
    assert_eq!(w.tick(), 1);
}

#[test]
fn another_servers_stuck_answers_are_never_notified_or_looked_up_here() {
    let mut w = World::new();
    let qid = w.ask_on(("w1:p1", "w1:t1"), Some("/tmp/other.sock"));
    w.answer(qid, "blocked");
    w.sql(
        "UPDATE questions SET answered_at = '2026-10-15T00:00:00+00:00', delivery_error = ?, \
         delivery_attempts = 2 WHERE id = ?",
        params![inbox_core::deliver::NOT_VERIFIED, qid],
    );
    w.fake.reset();
    w.tick();
    assert!(w.notes.is_empty());
    assert!(w.fake.stub.params_of("tab.get").is_empty());
    assert!(!w.store.notified(qid).unwrap()); // left for its own worker to notify
}

#[test]
fn a_legacy_row_whose_pane_never_holds_its_conversation_is_gone_after_ten_misses() {
    let mut w = World::new();
    let qid = w.pending_answer("blocked");
    w.sql(
        "UPDATE question_ext SET herdr_server = NULL WHERE id = ?",
        params![qid],
    );
    w.fake.set(|k| k.session = Some("someone-else".into()));
    w.sql(
        "UPDATE question_ext SET legacy_misses = 8, legacy_checked_at = NULL WHERE id = ?",
        params![qid],
    );
    w.tick(); // the ninth miss
    assert!(!w.store.agent_gone(qid).unwrap());
    w.sql(
        "UPDATE question_ext SET legacy_checked_at = NULL WHERE id = ?",
        params![qid],
    );
    assert_eq!(w.tick(), 0); // the tenth: gone, no longer pending
    assert!(w.store.agent_gone(qid).unwrap());
    assert_eq!(
        w.error(qid),
        "agent gone: no agent of that conversation in pane w1:p1"
    );
    assert_eq!(w.shown("gone").len(), 1); // and the operator hears of it once
}

#[test]
fn a_pane_with_answers_of_two_kinds_gets_one_notification_per_kind() {
    let mut w = World::new();
    let (blocked, busy) = (w.ask(), w.ask());
    for (qid, error) in [
        (
            blocked,
            "pending: blocked: the agent is at a prompt or dialog",
        ),
        (busy, "pending: busy: the agent is working"),
    ] {
        w.store.set_answer(qid, "yes", None).unwrap();
        w.sql(
            "UPDATE questions SET answered_at = '2026-10-15T00:00:00+00:00', delivery_error = ? WHERE id = ?",
            params![error, qid],
        );
    }
    let rows = w.store.stuck_answers().unwrap();
    w.with_ctx(|ctx, server| {
        herdr_inbox::worker::duties::stuck::notify_stuck(
            &mut herdr_inbox::worker::duties::Round { ctx, server },
            &rows,
        )
        .unwrap()
    });
    let titles: Vec<&str> = w.notes.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        [
            format!("waiting: my-tab is at a prompt (Q{blocked})"),
            format!("Q{busy} not delivered to my-tab")
        ]
    );
}

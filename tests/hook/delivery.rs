use herdr_inbox::clock::{now, now_minus};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::common::*;
use crate::run::World;

/// An answer the worker could not type: the agent was working (pending, busy grace).
fn answered_busy(db: &Connection, question: &str, text: &str) -> i64 {
    let id = ask(db, A, question);
    answer(db, id, text);
    db.execute(
        "UPDATE questions SET delivery_error = 'pending: busy: the agent is working' WHERE id = ?",
        [id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?)",
        params![id, id, now()],
    )
    .unwrap();
    id
}

fn row(db: &Connection, id: i64) -> (Option<String>, Option<String>, Option<String>) {
    db.query_row(
        "SELECT delivered_at, acknowledged_at, delivery_error FROM questions WHERE id = ?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

fn ext(db: &Connection, id: i64, column: &str) -> Option<String> {
    db.query_row(
        &format!("SELECT {column} FROM question_ext WHERE id = ?"),
        [id],
        |r| r.get(0),
    )
    .optional()
    .unwrap()
    .flatten()
}

#[test]
fn a_busy_sessions_answer_goes_out_as_hook_context_once() {
    // Python test_hook_delivery::test_a_busy_sessions_answer_goes_out_as_hook_context_once
    for (event, name) in [("post-tool-batch", "PostToolBatch"), ("stop", "Stop")] {
        let w = World::new();
        let db = open(&w.db);
        let id = answered_busy(&db, "Merge it?", "squash");
        let ctx = w.hook(event, &w.payload(json!({}))).context(name);
        assert_eq!(
            ctx,
            Some(format!("[inbox] Q{id} answered — Merge it?: squash"))
        );
        let (delivered, acked, error) = row(&db, id);
        assert!(
            delivered.is_some() && acked.is_none() && error.is_none(),
            "sent; seen only once the transcript shows it"
        );
        assert!(ext(&db, id, "hook_sent_at").is_some() && ext(&db, id, "pending_since").is_none());
        assert_eq!(
            w.hook(event, &w.payload(json!({}))).context(name),
            None,
            "exactly once"
        );
    }
}

#[test]
fn several_answers_go_in_one_message() {
    let w = World::new();
    let db = open(&w.db);
    let q1 = answered_busy(&db, "One?", "a");
    let q2 = answered_busy(&db, "Two?", "b");
    let ctx = w
        .hook("post-tool-batch", &w.payload(json!({})))
        .context("PostToolBatch");
    assert_eq!(
        ctx,
        Some(format!("[inbox] answered — Q{q1} One?: a · Q{q2} Two?: b"))
    );
}

#[test]
fn answers_over_the_cap_wait_for_the_next_firing_with_their_state_kept() {
    let w = World::new();
    let db = open(&w.db);
    let (a, b) = ("a".repeat(2500), "b".repeat(2500));
    let q1 = answered_busy(&db, "One?", &a);
    let q2 = answered_busy(&db, "Two?", &b);
    let first = w
        .hook("post-tool-batch", &w.payload(json!({})))
        .context("PostToolBatch")
        .unwrap();
    assert_eq!(first, format!("[inbox] Q{q1} answered — One?: {a}"));
    let (delivered, _, error) = row(&db, q2);
    assert_eq!(
        (delivered, error.as_deref()),
        (None, Some("pending: busy: the agent is working")),
        "claim given back untouched"
    );
    let second = w
        .hook("post-tool-batch", &w.payload(json!({})))
        .context("PostToolBatch")
        .unwrap();
    assert_eq!(second, format!("[inbox] Q{q2} answered — Two?: {b}"));
}

#[test]
fn nothing_for_a_subagent_another_session_or_outside_herdr() {
    let w = World::new();
    let db = open(&w.db);
    let id = answered_busy(&db, "Merge it?", "squash");
    assert_eq!(
        w.hook(
            "post-tool-batch",
            &w.payload(json!({"agent_id": "agent-1"}))
        )
        .context("PostToolBatch"),
        None
    );
    assert_eq!(
        w.hook("post-tool-batch", &w.payload(json!({"session_id": B})))
            .context("PostToolBatch"),
        None
    );
    assert_eq!(
        w.hook_in("post-tool-batch", &w.payload(json!({})), None)
            .context("PostToolBatch"),
        None
    );
    assert_eq!(row(&db, id).0, None);
    let stamped: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE hook_seen_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        stamped, 1,
        "only the other session's own firing stamped liveness; never a subagent's"
    );
}

#[test]
fn a_row_held_by_another_sender_or_already_seen_is_left_alone() {
    let w = World::new();
    let db = open(&w.db);
    let id = answered_busy(&db, "Merge it?", "squash");
    db.execute(
        "UPDATE questions SET delivery_error = ? WHERE id = ?",
        params![format!("sending:{}:other", now()), id],
    )
    .unwrap();
    assert_eq!(
        w.hook("post-tool-batch", &w.payload(json!({})))
            .context("PostToolBatch"),
        None
    );
    db.execute(
        "UPDATE questions SET delivery_error = NULL, acknowledged_at = ? WHERE id = ?",
        params![now(), id],
    )
    .unwrap();
    assert_eq!(
        w.hook("post-tool-batch", &w.payload(json!({})))
            .context("PostToolBatch"),
        None
    );
}

#[test]
fn an_answer_already_in_the_transcript_is_recorded_not_resent() {
    let w = World::new();
    let db = open(&w.db);
    let id = answered_busy(&db, "Merge it?", "squash");
    let record = json!({"type": "user", "timestamp": now(), "message": {"content": format!("[inbox] Q{id} answered — Merge it?: squash")}});
    std::fs::write(&w.transcript, format!("{record}\n")).unwrap();
    assert_eq!(
        w.hook("post-tool-batch", &w.payload(json!({})))
            .context("PostToolBatch"),
        None
    );
    let (delivered, acked, _) = row(&db, id);
    assert!(
        delivered.is_some() && acked.is_some(),
        "a late landing: verified"
    );
    assert!(ext(&db, id, "verified_at").is_some() && ext(&db, id, "hook_sent_at").is_none());
}

#[test]
fn only_the_delivery_hook_firing_marks_the_session_live_throttled() {
    let w = World::new();
    let db = open(&w.db);
    let seen = || -> Option<String> {
        db.query_row(
            "SELECT hook_seen_at FROM sessions WHERE session_id = ?",
            [A],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten()
    };
    w.hook("post-tool-batch", &w.payload(json!({})));
    let first = seen().expect("stamped");
    let recent = now_minus(std::time::Duration::from_secs(30));
    db.execute(
        "UPDATE sessions SET hook_seen_at = ? WHERE session_id = ?",
        params![recent, A],
    )
    .unwrap();
    w.hook("post-tool-batch", &w.payload(json!({})));
    assert_eq!(seen(), Some(recent), "within the minute: no write");
    db.execute(
        "UPDATE sessions SET hook_seen_at = ? WHERE session_id = ?",
        params![now_minus(std::time::Duration::from_secs(120)), A],
    )
    .unwrap();
    w.hook("stop", &w.payload(json!({})));
    assert!(seen().unwrap() >= first);
    let transcript: Option<String> = db
        .query_row(
            "SELECT transcript_path FROM sessions WHERE session_id = ?",
            [A],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        transcript, None,
        "a session without answers writes only the liveness stamp"
    );
}

#[test]
fn a_hook_on_a_pre_bridge_stamp_delivers_and_leaves_the_stamp() {
    for (event, name) in [("post-tool-batch", "PostToolBatch"), ("stop", "Stop")] {
        let w = World::new();
        let db = open(&w.db);
        let id = answered_busy(&db, "Merge it?", "squash");
        db.pragma_update(None, "user_version", 1_668_105_336_i64)
            .unwrap();
        let ctx = w.hook(event, &w.payload(json!({}))).context(name);
        assert_eq!(
            ctx,
            Some(format!("[inbox] Q{id} answered — Merge it?: squash"))
        );
        let stamp: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stamp, 1_668_105_336, "{event}");
    }
}

#[test]
fn a_hook_on_an_older_schema_does_nothing_and_exits_0() {
    let w = World::new();
    let db = open(&w.db);
    let id = answered_busy(&db, "Merge it?", "squash");
    db.execute_batch("DROP INDEX alerts_status; PRAGMA user_version = 1668105336")
        .unwrap();
    for event in ["post-tool-batch", "stop"] {
        let r = w.hook(event, &w.payload(json!({})));
        assert_eq!((r.code, r.stdout.as_str()), (0, ""), "{event}");
    }
    let (delivered, _, error) = row(&db, id);
    assert_eq!(
        (delivered, error.as_deref()),
        (None, Some("pending: busy: the agent is working")),
        "never claimed"
    );
    let stamp: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamp, 1_668_105_336);
}

#[test]
fn a_predecessors_answer_rides_the_continued_sessions_hook() {
    // the conversation moved from B to A (B's transcript ends in continued-in A)
    let w = World::new();
    let db = open(&w.db);
    let id = ask(&db, B, "Asked before the move?");
    answer(&db, id, "yes");
    db.execute(
        "INSERT INTO sessions (session_id, continued_in) VALUES (?, ?)",
        [B, A],
    )
    .unwrap();
    let record = json!({"type": "continued-in", "sessionId": B, "continuedInSessionId": A, "timestamp": now()});
    std::fs::write(
        w.transcript.with_file_name(format!("{B}.jsonl")),
        format!("{record}\n"),
    )
    .unwrap();
    let ctx = w
        .hook("post-tool-batch", &w.payload(json!({})))
        .context("PostToolBatch");
    assert_eq!(
        ctx,
        Some(format!(
            "[inbox] Q{id} answered — Asked before the move?: yes"
        ))
    );
}

#[test]
fn a_database_held_by_a_writer_costs_this_firing_and_nothing_else() {
    // a writer holds the lock past the 0.5 s busy timeout: nothing printed, nothing changed, exit 0
    let w = World::new();
    let db = open(&w.db);
    let id = answered_busy(&db, "Merge it?", "squash");
    let lock = locked(&w.db, "IMMEDIATE");
    let (ran, took) = timed(|| w.hook("post-tool-batch", &w.payload(json!({}))));
    lock.execute_batch("ROLLBACK").unwrap();
    assert_eq!((ran.code, ran.stdout.as_str()), (0, ""));
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    let (delivered, _, error) = row(&db, id);
    assert_eq!(
        (delivered, error.as_deref()),
        (None, Some("pending: busy: the agent is working")),
        "never claimed"
    );
    let ctx = w
        .hook("post-tool-batch", &w.payload(json!({})))
        .context("PostToolBatch");
    assert_eq!(
        ctx,
        Some(format!("[inbox] Q{id} answered — Merge it?: squash")),
        "the next firing carries it"
    );
}

#[test]
fn liveness_is_stamped_after_own_session_on_the_tool_hook_path_only() {
    // acceptance check (#65 final pass L2): hook_seen_at only from PostToolBatch/Stop of the
    // session's own hook; never from SessionStart, SubagentStart or a subagent's firing
    let w = World::new();
    let db = open(&w.db);
    let stamped = || -> i64 {
        db.query_row(
            "SELECT COUNT(*) FROM sessions WHERE hook_seen_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    w.hook("session-start", &w.payload(json!({"source": "startup"})));
    w.hook("subagent-start", &w.payload(json!({"agent_id": "a1"})));
    w.hook("post-tool-batch", &w.payload(json!({"agent_id": "a1"})));
    w.hook("stop", &w.payload(json!({"agent_id": "a1"})));
    assert_eq!(stamped(), 0);
    w.hook("stop", &w.payload(json!({})));
    assert_eq!(stamped(), 1);
}

#[test]
fn a_held_database_costs_one_busy_wait_not_one_per_answer() {
    // review #66 H1: 12 answers, no transcript_path (record_session is a no-op), a writer holding
    // the lock: the first failed claim stops the claiming (Python: 0.81 s; one wait per answer: 6 s)
    let w = World::new();
    let db = open(&w.db);
    for i in 0..12 {
        answered_busy(&db, &format!("Q{i}?"), "a");
    }
    let lock = locked(&w.db, "IMMEDIATE");
    let (ran, took) = timed(|| w.hook("post-tool-batch", &json!({"session_id": A}).to_string()));
    lock.execute_batch("ROLLBACK").unwrap();
    assert_eq!((ran.code, ran.stdout.as_str()), (0, ""));
    assert!(took < std::time::Duration::from_millis(1500), "{took:?}");
}

#[test]
fn a_lone_surrogate_in_the_payload_still_delivers() {
    // review #66 M1: a tool_response cut inside an emoji (Python's json.loads accepts it)
    let w = World::new();
    let db = open(&w.db);
    let id = answered_busy(&db, "Merge it?", "squash");
    let payload = w
        .payload(json!({}))
        .replacen('{', r#"{"tool_response": "cut \ud83d", "#, 1);
    let ctx = w.hook("post-tool-batch", &payload).context("PostToolBatch");
    assert_eq!(
        ctx,
        Some(format!("[inbox] Q{id} answered — Merge it?: squash"))
    );
}

#[test]
fn an_answer_whose_claim_is_lost_before_the_record_is_never_printed() {
    // the exactly-once rebuild: a trigger steals Q2's claim right after this hook took it
    // (another sender's live claim), so its record fails and the message is rebuilt from Q1
    let w = World::new();
    let db = open(&w.db);
    let q1 = answered_busy(&db, "One?", "a");
    let q2 = answered_busy(&db, "Two?", "b");
    let thief = format!("sending:{}:7h1ef000", now());
    db.execute_batch(&format!(
        "CREATE TRIGGER steal AFTER UPDATE OF delivery_error ON questions \
         WHEN NEW.id = {q2} AND NEW.delivery_error LIKE 'sending:%' AND NEW.delivery_error NOT LIKE '%:7h1ef000' \
         BEGIN UPDATE questions SET delivery_error = '{thief}' WHERE id = {q2}; END"
    ))
    .unwrap();
    let ctx = w
        .hook("post-tool-batch", &w.payload(json!({})))
        .context("PostToolBatch");
    assert_eq!(
        ctx,
        Some(format!("[inbox] Q{q1} answered — One?: a")),
        "never Q2: its record lost the claim"
    );
    let (delivered, _, error) = row(&db, q2);
    assert_eq!(
        (delivered, error),
        (None, Some(thief)),
        "Q2 stays with the other sender"
    );
}

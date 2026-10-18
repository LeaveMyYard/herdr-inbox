use std::cell::Cell;

use herdr_agent_kit::plugin_env::LockFile;
use herdr_inbox::hook::payload::Payload;
use herdr_inbox::rewake::run;
use inbox_core::rewake::{PANE_CHECK_EVERY, UNKNOWN_SESSION_END, WATCH_MAX_SECONDS};
use rusqlite::OptionalExtension;

use crate::common::A;
use crate::world::{FakeClock, World};

const POLL: f64 = 2.0;

fn rewrite_owner(w: &World) {
    std::fs::write(
        w.dir.path().join("rewake").join(format!("{A}.owner")),
        "newer",
    )
    .unwrap();
}

#[test]
fn an_idle_session_is_woken_with_its_answer_once() {
    let w = World::new();
    let id = w.pending_answer();
    let clock = FakeClock::new(|_, _| {});
    assert_eq!(
        w.watch(&clock),
        Some(format!("[inbox] Q{id} answered — Which colour?: amber"))
    );
    assert_eq!(clock.sleeps.get(), 0, "the first poll looks at once");
    let (acked, sent): (Option<String>, Option<String>) = w
        .conn()
        .query_row(
            "SELECT q.acknowledged_at, e.hook_sent_at FROM questions q JOIN question_ext e ON e.id = q.id WHERE q.id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(w.delivered(id) && acked.is_none() && sent.is_some());
    assert!(
        w.herdr.params_of("agent.prompt").is_empty(),
        "nothing typed"
    );
    let lock = LockFile::new(w.dir.path().join("rewake").join(format!("{A}.lock")));
    assert!(
        lock.try_exclusive().unwrap().is_some(),
        "the lock goes with the watcher"
    );
    assert_eq!(
        w.watch(&FakeClock::new(|n, t| if n == 1 {
            t.set(WATCH_MAX_SECONDS)
        })),
        None,
        "once"
    );
}

#[test]
fn a_working_or_blocked_session_is_not_woken_until_it_rests() {
    let w = World::new();
    let id = w.pending_answer();
    w.agent(Some(A), "working");
    let clock = FakeClock::new(|n, _| match n {
        3 => w.agent(Some(A), "blocked"),
        5 => w.agent(Some(A), "done"),
        _ => {}
    });
    assert!(w.watch(&clock).is_some());
    assert_eq!(clock.sleeps.get(), 5);
    assert!(w.delivered(id));
}

#[test]
fn a_pane_holding_another_conversation_ends_the_watch() {
    let w = World::new();
    let id = w.pending_answer();
    w.agent(Some("dddddddd-0000-4000-8000-00000000000d"), "idle");
    assert_eq!(w.watch(&FakeClock::new(|_, _| {})), None);
    assert!(!w.delivered(id));
}

#[test]
fn a_newer_watcher_supersedes_the_old_one() {
    let w = World::new();
    let clock = FakeClock::new(|n, _| {
        if n == 2 {
            rewrite_owner(&w)
        }
    });
    assert_eq!(w.watch(&clock), None);
    assert_eq!(clock.sleeps.get(), 2);
}

#[test]
fn the_watch_ends_with_its_claude_process_and_after_the_max_age() {
    let w = World::new();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let clock = FakeClock::new(|_, _| {});
    let mut watcher = w.watcher(&clock, Some("w1:p1"));
    watcher.owner = Some(dead);
    assert_eq!(run(&watcher, &w.payload()), None);
    assert_eq!(clock.sleeps.get(), 0);
    let clock = FakeClock::new(|n, t| {
        if n == 1 {
            t.set(WATCH_MAX_SECONDS - 1.0)
        }
    });
    assert_eq!(w.watch(&clock), None);
    assert!(clock.t.get() >= WATCH_MAX_SECONDS && clock.t.get() <= WATCH_MAX_SECONDS + POLL);
}

#[test]
fn nothing_for_a_subagent_outside_herdr_or_a_bad_session_id() {
    let w = World::new();
    let id = w.pending_answer();
    let clock = FakeClock::new(|_, _| {});
    let subagent = Payload {
        agent_id: true,
        ..w.payload()
    };
    assert_eq!(run(&w.watcher(&clock, Some("w1:p1")), &subagent), None);
    assert_eq!(run(&w.watcher(&clock, None), &w.payload()), None);
    let bad = Payload {
        session_id: Some("../x".into()),
        ..Payload::default()
    };
    assert_eq!(run(&w.watcher(&clock, Some("w1:p1")), &bad), None);
    assert!(!w.delivered(id) && w.agent_gets() == 0);
    assert!(
        !w.dir.path().join("rewake").exists(),
        "no owner or lock file for them"
    );
}

#[test]
fn the_waiting_watcher_stamps_its_own_column_never_the_tool_hooks() {
    let w = World::new();
    let seen = Cell::new(None);
    let clock = FakeClock::new(|n, _| {
        let row: Option<(Option<String>, Option<String>)> = w
            .conn()
            .query_row(
                "SELECT hook_seen_at, watch_seen_at FROM sessions WHERE session_id = ?",
                [A],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap();
        if n == 1 {
            seen.set(row);
        }
        if n == 2 {
            rewrite_owner(&w);
        }
    });
    assert_eq!(w.watch(&clock), None);
    let (hook, watch) = seen.take().expect("a sessions row");
    assert!(hook.is_none() && watch.is_some());
}

#[test]
fn a_watch_left_behind_by_clear_ends_without_any_answer() {
    let w = World::new();
    w.agent(Some("eeeeeeee-0000-4000-8000-00000000000e"), "idle");
    let clock = FakeClock::new(|_, _| {});
    assert_eq!(w.watch(&clock), None);
    assert!(clock.t.get() >= PANE_CHECK_EVERY && clock.t.get() <= PANE_CHECK_EVERY + 2.0 * POLL);
    assert_eq!(w.agent_gets(), 1, "one herdr look, not one per poll");
}

#[test]
fn a_pane_without_a_session_is_never_woken_and_the_watch_ends() {
    let w = World::new();
    let id = w.pending_answer();
    w.agent(None, "idle");
    let clock = FakeClock::new(|_, _| {});
    assert_eq!(w.watch(&clock), None);
    assert!(
        clock.t.get() >= UNKNOWN_SESSION_END && clock.t.get() <= UNKNOWN_SESSION_END + 2.0 * POLL
    );
    assert!(!w.delivered(id));
}

#[test]
fn the_session_reappearing_resumes_the_watch() {
    let w = World::new();
    let id = w.pending_answer();
    w.agent(None, "idle");
    let clock = FakeClock::new(|n, _| {
        if n == 5 {
            w.agent(Some(A), "idle")
        }
    });
    assert!(w.watch(&clock).is_some() && w.delivered(id));
}

#[test]
fn a_watcher_delivery_never_writes_hook_seen_at() {
    let w = World::new();
    w.pending_answer();
    assert!(w.watch(&FakeClock::new(|_, _| {})).is_some());
    let hook: Option<String> = w
        .conn()
        .query_row(
            "SELECT hook_seen_at FROM sessions WHERE session_id = ?",
            [A],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten();
    assert_eq!(hook, None);
}

#[test]
fn the_watcher_on_an_older_schema_waits_for_the_worker_to_migrate() {
    let w = World::new();
    let id = w.pending_answer();
    w.conn()
        .execute_batch("DROP INDEX alerts_status; PRAGMA user_version = 1668105336")
        .unwrap();
    let clock = FakeClock::new(|n, _| {
        if n == 3 {
            w.conn()
                .execute_batch("CREATE INDEX alerts_status ON alerts(status)")
                .unwrap(); // "migrated"
        }
    });
    assert!(
        w.watch(&clock).is_some(),
        "it neither migrates nor ends the watch"
    );
    assert_eq!(clock.sleeps.get(), 3);
    assert!(w.delivered(id));
    let stamp: i64 = w
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamp, 1_668_105_336, "never re-stamped");
}

#[test]
fn an_unreachable_herdr_ends_the_watch() {
    let w = World::new();
    w.pending_answer();
    let clock = FakeClock::new(|_, _| {});
    let mut watcher = w.watcher(&clock, Some("w1:p1"));
    watcher.herdr = herdr_agent_kit::herdr_api::blocking::Herdr::new(
        herdr_agent_kit::herdr_api::ServerId::new(w.dir.path().join("no.sock")),
    );
    assert_eq!(run(&watcher, &w.payload()), None);
}

#[test]
fn the_old_sessions_watcher_ends_and_the_new_one_wakes() {
    // Python test_session_continuation: herdr keeps reporting OLD after OLD continued in A
    let w = World::new();
    let old = "0ld00000-0000-4000-8000-000000000001".replace("0ld", "01d");
    let record = serde_json::json!({"type": "continued-in", "sessionId": old, "continuedInSessionId": A,
                                    "timestamp": "2026-10-17T10:00:00Z"});
    std::fs::write(
        w.dir
            .path()
            .join("projects")
            .join("-r")
            .join(format!("{old}.jsonl")),
        format!("{record}\n"),
    )
    .unwrap();
    w.agent(Some(&old), "idle");
    let id = w.pending_answer();
    let old_payload = Payload {
        session_id: Some(old.clone()),
        ..Payload::default()
    };
    assert_eq!(
        run(
            &w.watcher(&FakeClock::new(|_, _| {}), Some("w1:p1")),
            &old_payload
        ),
        None
    );
    assert!(
        w.watch(&FakeClock::new(|_, _| {})).is_some(),
        "A runs in the pane: woken"
    );
    assert!(w.delivered(id));
}

#[test]
fn the_watcher_on_a_pre_bridge_stamp_wakes_without_any_schema_work() {
    let w = World::new();
    let id = w.pending_answer();
    w.conn()
        .pragma_update(None, "user_version", 1_668_105_336_i64)
        .unwrap();
    let text = w.watch(&FakeClock::new(|_, _| {}));
    assert_eq!(
        text,
        Some(format!("[inbox] Q{id} answered — Which colour?: amber"))
    );
    let stamp: i64 = w
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamp, 1_668_105_336);
}

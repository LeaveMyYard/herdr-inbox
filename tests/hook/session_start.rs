use herdr_inbox::hook::rules::{SESSION_RULES, SUBAGENT_RULES};
use serde_json::json;

use crate::common::*;
use crate::run::World;

fn seed(w: &World) -> (i64, i64) {
    let db = open(&w.db);
    let open_q = ask(&db, A, "Which region?");
    let answered = ask(&db, A, "Squash or merge?");
    answer(&db, answered, "squash");
    ask(&db, B, "Other session question?");
    (open_q, answered)
}

#[test]
fn startup_injects_the_rules_and_lists_nothing() {
    // Python test_hook::test_startup_injects_rules, test_startup_does_not_list_questions
    let w = World::new();
    seed(&w);
    let ctx = w
        .hook("session-start", &w.payload(json!({"source": "startup"})))
        .context("SessionStart");
    assert_eq!(ctx.as_deref(), Some(SESSION_RULES));
}

#[test]
fn compact_lists_this_sessions_pending_questions() {
    // Python test_hook::test_compact_lists_this_sessions_pending_questions
    let w = World::new();
    let (q1, q2) = seed(&w);
    let ctx = w
        .hook("session-start", &w.payload(json!({"source": "compact"})))
        .context("SessionStart")
        .unwrap();
    assert!(ctx.starts_with(SESSION_RULES));
    assert!(ctx.contains(&format!("Q{q1} (open): Which region?")));
    assert!(ctx.contains(&format!(
        "Q{q2} (answered, not yet seen): Squash or merge? -> squash"
    )));
    assert!(!ctx.contains("Other session question"));
    let none = w.hook(
        "session-start",
        &json!({"session_id": "nobody", "source": "compact"}).to_string(),
    );
    assert_eq!(
        none.context("SessionStart").as_deref(),
        Some(SESSION_RULES),
        "test_compact_without_questions_is_just_the_rules"
    );
}

#[test]
fn an_unusable_or_older_database_costs_the_list_never_the_rules() {
    // Python test_hook::test_broken_db_never_fails, test_hook_delivery::test_a_hook_on_an_older_schema_does_nothing_and_exits_0
    let w = World::new();
    std::fs::write(&w.db, "not a database").unwrap();
    let ctx = w
        .hook("session-start", &w.payload(json!({"source": "compact"})))
        .context("SessionStart");
    assert_eq!(ctx.as_deref(), Some(SESSION_RULES));
    let old = World::new();
    open(&old.db)
        .execute_batch("DROP VIEW questions_v6; PRAGMA user_version = 1668105336")
        .unwrap();
    let before = std::fs::read(&old.db).unwrap();
    let ctx = old
        .hook("session-start", &old.payload(json!({"source": "startup"})))
        .context("SessionStart");
    assert_eq!(ctx.as_deref(), Some(SESSION_RULES));
    assert_eq!(
        std::fs::read(&old.db).unwrap(),
        before,
        "no migration, no stamp, no transcript record"
    );
}

#[test]
fn the_transcript_path_is_remembered_after_the_rules() {
    // Python test_verified_delivery::test_hook_records_the_transcript_path_used_for_verification
    let w = World::new();
    w.hook("session-start", &w.payload(json!({"source": "startup"})));
    let path: String = open(&w.db)
        .query_row(
            "SELECT transcript_path FROM sessions WHERE session_id = ?",
            [A],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(path, w.transcript.to_str().unwrap());
    let seen: Option<String> = open(&w.db)
        .query_row(
            "SELECT hook_seen_at FROM sessions WHERE session_id = ?",
            [A],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        seen, None,
        "SessionStart says nothing about the delivery hooks firing (#26 M1)"
    );
}

#[test]
fn only_a_resume_writes_a_start_marker_with_its_pane() {
    // Python test_older_conversation::test_final_m1_only_a_resume_writes_a_start_marker
    let w = World::new();
    let marker = w.state().join("starts").join(A);
    for source in ["startup", "clear", "compact", "fork"] {
        w.hook_in(
            "session-start",
            &w.payload(json!({"source": source})),
            Some("w1:p7"),
        );
        assert!(!marker.exists(), "{source}");
    }
    w.hook_in(
        "session-start",
        &w.payload(json!({"source": "resume"})),
        Some("w1:p7"),
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "w1:p7");
}

#[test]
fn subagents_get_their_rules_and_garbage_or_no_pane_gets_nothing() {
    // Python test_hook::test_subagent_start, test_garbage_input_never_fails
    let w = World::new();
    let ctx = w.hook(
        "subagent-start",
        &json!({"agent_id": "a1", "agent_type": "Explore"}).to_string(),
    );
    assert_eq!(
        ctx.context("SubagentStart").as_deref(),
        Some(SUBAGENT_RULES)
    );
    let bad = w.hook("session-start", "{not json");
    assert_eq!((bad.code, bad.stdout.as_str()), (0, ""));
    let outside = w.hook_in(
        "session-start",
        &w.payload(json!({"source": "startup"})),
        None,
    );
    assert_eq!(
        (outside.code, outside.stdout.as_str()),
        (0, ""),
        "not a herdr pane's agent"
    );
    let r = w.hook("no-such-event", "{}");
    assert_eq!((r.code, r.stdout.as_str()), (0, ""));
}

#[test]
fn bad_arguments_exit_0() {
    // Python test_hook_launch::test_hook_crash_and_bad_arguments_exit_0
    for args in [vec!["hook"], vec!["hook", "stop", "extra"]] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_herdr-inbox"))
            .args(&args)
            .env_clear()
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            (out.status.code(), out.stdout.len()),
            (Some(0), 0),
            "{args:?}"
        );
    }
}

#[test]
fn a_json_payload_that_is_not_an_object_still_gets_the_rules() {
    // review #66 L1: Python fell back to the bare rules (session-start) and never read the
    // payload (subagent-start)
    let w = World::new();
    for raw in ["[]", "1"] {
        assert_eq!(
            w.hook("session-start", raw)
                .context("SessionStart")
                .as_deref(),
            Some(SESSION_RULES)
        );
        assert_eq!(
            w.hook("subagent-start", raw)
                .context("SubagentStart")
                .as_deref(),
            Some(SUBAGENT_RULES)
        );
        let r = w.hook("stop", raw);
        assert_eq!((r.code, r.stdout.as_str()), (0, ""));
    }
}

/// A Claude Code process stand-in: a script named `claude` (its comm on Linux) that runs `next`.
#[cfg(target_os = "linux")]
fn claude_script(dir: &std::path::Path, next: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("claude");
    std::fs::write(&path, format!("#!/bin/sh\n{next}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(target_os = "linux")]
#[test]
fn a_claude_nested_in_another_gets_nothing() {
    // `claude -p` started from an agent's Bash tool inherits the pane's HERDR_PANE_ID: it is not
    // the pane's agent (Python proc.nested_claude: two Claude processes above the hook)
    let w = World::new();
    let run = |nested: bool| {
        let bin = env!("CARGO_BIN_EXE_herdr-inbox");
        let inner = claude_script(
            &w.state().join(if nested { "n/inner" } else { "o/inner" }),
            &format!("{bin} hook subagent-start"),
        );
        let top = if nested {
            claude_script(&w.state().join("n/outer"), &inner.display().to_string())
        } else {
            inner
        };
        // detached (a background subshell whose parent exits): the walk up from the hook meets
        // only these scripts, never the Claude Code that may be running this test
        let out = w.state().join(if nested { "n/out" } else { "o/out" });
        let done = out.with_extension("done");
        let script = format!(
            "(\"{}\" > \"{}\" 2>/dev/null; touch \"{}\") </dev/null &",
            top.display(),
            out.display(),
            done.display()
        );
        std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HERDR_PANE_ID", "w1:p1")
            .env("HERDR_INBOX_DB", &w.db)
            .status()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::read_to_string(&out).unwrap()
    };
    assert!(
        run(false).contains("SubagentStart"),
        "one Claude above: its own hook"
    );
    assert_eq!(run(true), "", "a Claude inside a Claude: nothing");
}

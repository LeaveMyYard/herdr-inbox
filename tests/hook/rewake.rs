use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use serde_json::json;

use crate::common::*;
use crate::run::World;

#[test]
fn the_watcher_prints_the_answer_and_exits_with_the_wake_code() {
    let w = World::new();
    let db = open(&w.db);
    let id = ask(&db, A, "Which colour?");
    answer(&db, id, "amber");
    let herdr = FakeHerdr::start();
    let agent = json!({"agent": "claude", "agent_status": "idle", "pane_id": "w1:p1", "agent_session": {"value": A}});
    herdr.on("agent.get", Reply::Result(json!({"agent": agent})));
    let r = w.hook_env(
        "rewake-watch",
        &w.payload(json!({})),
        Some("w1:p1"),
        &[("HERDR_SOCKET_PATH", herdr.socket())],
    );
    assert_eq!(
        r.code, 3,
        "the wrapper maps exactly 3 to Claude Code's exit 2"
    );
    assert_eq!(
        r.stdout,
        format!("[inbox] Q{id} answered — Which colour?: amber\n"),
        "the text itself, not JSON"
    );
    assert_eq!(herdr.params_of("agent.get"), [json!({"target": "w1:p1"})]);
}

#[test]
fn a_watcher_that_cannot_start_exits_0_silently() {
    let w = World::new();
    for payload in [
        w.payload(json!({"agent_id": "a1"})),
        json!({"session_id": "not-a-uuid"}).to_string(),
        "{not json".into(),
    ] {
        let r = w.hook("rewake-watch", &payload);
        assert_eq!((r.code, r.stdout.as_str()), (0, ""), "{payload}");
    }
}

#[test]
fn a_wake_whose_text_cannot_be_written_exits_0() {
    // review #67 L2: never a wake without its text (Python returns 0)
    let w = World::new();
    let db = open(&w.db);
    let id = ask(&db, A, "Which colour?");
    answer(&db, id, "amber");
    let herdr = FakeHerdr::start();
    let agent = json!({"agent": "claude", "agent_status": "idle", "pane_id": "w1:p1", "agent_session": {"value": A}});
    herdr.on("agent.get", Reply::Result(json!({"agent": agent})));
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader); // stdout goes nowhere: the write fails (EPIPE)
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_herdr-inbox"))
        .args(["hook", "rewake-watch"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", w.state())
        .env("HERDR_PANE_ID", "w1:p1")
        .env("HERDR_INBOX_DB", &w.db)
        .env("XDG_STATE_HOME", w.state().join("state"))
        .env("CLAUDE_CONFIG_DIR", w.state().join("claude"))
        .env("HERDR_SOCKET_PATH", herdr.socket())
        .stdin(std::process::Stdio::piped())
        .stdout(writer)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(w.payload(json!({})).as_bytes())
        .unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

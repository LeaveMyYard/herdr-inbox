//! The `herdr-inbox worker` process and the CLI starting one (Python
//! test_delivery_worker.py `test_worker_process_runs_and_exits_when_idle`,
//! `test_every_answer_or_send_ensures_the_worker`). Each process runs
//! with only the test's own environment, against the fake herdr's socket
//! and the test's database. A worker this test started directly is killed
//! if it overstays; one the CLI started ends by itself (it idles out), and
//! the test waits for its lock to be free.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use herdr_agent_kit::internal::plugin_paths::inbox::server_key;
use herdr_inbox::worker::lock::is_running;

use crate::World;

impl World {
    fn command(&self, args: &[&str]) -> Command {
        let d = self.dir.path();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-inbox"));
        cmd.args(args)
            .env_clear()
            .env("HOME", d)
            .env("PATH", "/usr/bin:/bin")
            .env("XDG_STATE_HOME", d.join("state"))
            .env("XDG_CONFIG_HOME", d.join("xdg"))
            .env("HERDR_SOCKET_PATH", &self.fake.stub.socket)
            .env("HERDR_BIN_PATH", d.join("no-herdr"))
            .env("HERDR_INBOX_DB", &self.db)
            .env("HERDR_PLUGIN_CONFIG_DIR", d.join("cfg"))
            .env("CLAUDE_CONFIG_DIR", d.join("claude"))
            .env("INBOX_SSH", d.join("no-ssh"))
            .env("HERDR_INBOX_WORKER_TICK", "0.1")
            .env("HERDR_INBOX_WORKER_IDLE_EXIT", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    }

    fn lock(&self) -> PathBuf {
        let key = server_key(OsStr::new(&self.fake.stub.socket));
        self.db.parent().unwrap().join(format!("worker-{key}.lock"))
    }
}

#[path = "session.rs"]
mod session;

/// Waits up to `secs` for `child` to exit; kills it (our own child) if not.
fn wait(mut child: Child, secs: u64) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[test]
fn the_worker_process_delivers_what_is_pending_and_exits_when_idle() {
    let mut w = World::new();
    let qid = w.pending_answer("blocked");
    assert!(w.store.get(qid).unwrap().unwrap().verified_at.is_none());
    w.fake.set(|k| k.status = "idle".into());
    let child = w.command(&["worker"]).spawn().unwrap();
    assert_eq!(wait(child, 60), Some(0), "the worker did not idle out");
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.verified_at.is_some(), "{q:?}");
    assert_eq!(w.fake.prompts().len(), 1);
    assert!(!is_running(&w.lock()));
}

#[test]
fn a_bad_worker_argument_is_a_usage_error() {
    let w = World::new();
    let child = w.command(&["worker", "--bogus"]).spawn().unwrap();
    assert_eq!(wait(child, 30), Some(2));
}

/// `ensure`'s spawn stamp beside `lock`: this revision started that worker.
fn stamp_of(lock: &Path) -> PathBuf {
    lock.with_file_name(format!(
        "{}.spawned-{}",
        lock.file_name().unwrap().to_string_lossy(),
        herdr_inbox::store::revision::MIGRATION_REVISION
    ))
}

/// A worker started from a test idles out by itself (IDLE_EXIT 1 s, inherited)
/// and lets its lock go.
fn wait_for_the_end(lock: &Path) {
    let stamp = stamp_of(lock);
    let deadline = Instant::now() + Duration::from_secs(60);
    while (is_running(lock) || stamp_fresh(&stamp)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!is_running(lock), "the started worker did not end");
}

#[test]
fn every_cli_answer_ensures_the_worker() {
    let mut w = World::new();
    let qid = w.ask();
    let id = format!("Q{qid}");
    let out = w.command(&["answer", &id, "yes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(stamp_of(&w.lock()).exists(), "no worker was started");
    wait_for_the_end(&w.lock());
    assert!(w.store.get(qid).unwrap().unwrap().verified_at.is_some());
}

#[test]
fn worker_ensure_starts_one_and_returns() {
    let w = World::new();
    let out = w.command(&["worker", "--ensure"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(stamp_of(&w.lock()).exists(), "no worker was started");
    wait_for_the_end(&w.lock());
}

#[test]
fn a_cli_answer_of_another_server_starts_that_servers_worker() {
    let mut w = World::new();
    let other = w.dir.path().join("other.sock");
    let qid = w.ask_on(("w1:p1", "w1:t1"), Some(&other.to_string_lossy()));
    let id = format!("Q{qid}");
    let out = w.command(&["answer", &id, "yes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let key = server_key(other.as_os_str());
    let lock = w.db.parent().unwrap().join(format!("worker-{key}.lock"));
    assert!(stamp_of(&lock).exists(), "no worker for the other server");
    wait_for_the_end(&lock);
    wait_for_the_end(&w.lock());
}

#[cfg(target_os = "linux")]
#[test]
fn a_warning_that_cannot_be_written_never_ends_the_worker() {
    let w = World::new();
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    // the database path is a directory: the migration warns, and the tick; the server
    // is gone, so the worker then ends by itself
    let db = w.dir.path().join("state-dir").join("inbox.db");
    std::fs::create_dir_all(&db).unwrap();
    let child = w
        .command(&["worker"])
        .env("HERDR_INBOX_DB", &db)
        .env("HERDR_SOCKET_PATH", w.dir.path().join("gone.sock"))
        .stderr(full)
        .spawn()
        .unwrap();
    assert_eq!(wait(child, 60), Some(0));
}

/// The stamp is under a second old: the worker may not hold the lock yet.
fn stamp_fresh(stamp: &std::path::Path) -> bool {
    std::fs::metadata(stamp)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < Duration::from_secs(1)))
}

#[test]
fn no_worker_keeps_the_cli_from_starting_one() {
    let mut w = World::new();
    let qid = w.ask();
    let id = format!("Q{qid}");
    let out = w
        .command(&["answer", &id, "yes"])
        .env("HERDR_INBOX_NO_WORKER", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let spawned = std::fs::read_dir(w.db.parent().unwrap())
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains(".spawned-"));
    assert!(!spawned);
}

/// The `tools/call` of `list_my_questions` a fresh MCP session sends.
const MCP_SESSION: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
    "\n",
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    "\n",
    r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_my_questions","arguments":{}}}"#,
    "\n",
);

#[test]
fn an_mcp_server_starting_on_a_ready_database_starts_the_worker() {
    let w = World::new();
    let mut child = w.command(&["mcp"]).stdin(Stdio::piped()).spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    let stamp = stamp_of(&w.lock());
    let deadline = Instant::now() + Duration::from_secs(30);
    while !stamp.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(stamp.exists(), "the MCP server started no worker");
    drop(stdin); // the client goes: the server ends
    assert!(wait(child, 30).is_some(), "the MCP server did not end");
    wait_for_the_end(&w.lock());
}

#[test]
fn an_mcp_call_on_a_database_not_ready_starts_the_worker() {
    let w = World::new();
    // never migrated: the store refuses it, and only a worker migrates
    let db = w.dir.path().join("fresh").join("inbox.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let lock = db.parent().unwrap().join(w.lock().file_name().unwrap());
    let stamp = stamp_of(&lock);
    let mut child = w
        .command(&["mcp"])
        .env("HERDR_INBOX_DB", &db)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(MCP_SESSION.as_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !stamp.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(stamp.exists(), "the MCP server started no worker");
    drop(stdin); // the client goes: the server ends
    assert!(wait(child, 30).is_some(), "the MCP server did not end");
    wait_for_the_end(&lock);
}

fn trace_steps(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_delivery_and_its_worker_leave_their_steps_in_the_trace_and_no_text() {
    let mut w = World::new();
    let qid = w.ask();
    let question = w.store.get(qid).unwrap().unwrap().question;
    let log = w.dir.path().join("trace.jsonl");
    let id = format!("Q{qid}");
    let status = w
        .command(&["answer", &id, "Answer Text 501"])
        .env("INBOX_TRACE", &log)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    wait_for_the_end(&w.lock());
    let steps = trace_steps(&log);
    let named = |name: &str| steps.iter().find(|s| s["step"] == name);
    let typed = named("deliver.prompt").expect("the answer was typed");
    assert!(typed["chars"].as_u64().unwrap() > 0, "{typed}");
    for step in [
        "herdr agent prompt",
        "herdr agent get",
        "herdr agent list",
        "deliver.verify_wait",
        "store.reconcile",
    ] {
        assert!(named(step).is_some(), "{step}: {steps:?}");
    }
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(!text.contains("Answer Text 501") && !text.contains(&question));
}

#[test]
fn only_a_database_stamped_older_is_checked() {
    let w = World::new();
    let log = w.dir.path().join("trace.jsonl");
    let list = || {
        let status = w
            .command(&["list"])
            .env("INBOX_TRACE", &log)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(0));
        trace_steps(&log)
            .iter()
            .filter(|s| s["step"] == "store.schema_check")
            .count()
    };
    assert_eq!(list(), 0);
    w.store.db().pragma_update(None, "user_version", 0).unwrap();
    assert_eq!(list(), 1);
}

//! The `herdr-inbox worker` process and the CLI starting one (Python
//! test_delivery_worker.py `test_worker_process_runs_and_exits_when_idle`,
//! `test_every_answer_or_send_ensures_the_worker`). Each process runs
//! with only the test's own environment, against the fake herdr's socket
//! and the test's database. A worker this test started directly is killed
//! if it overstays; one the CLI started ends by itself (it idles out), and
//! the test waits for its lock to be free.

use std::ffi::OsStr;
use std::path::PathBuf;
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

#[test]
fn every_cli_answer_ensures_the_worker() {
    let mut w = World::new();
    let qid = w.ask();
    let id = format!("Q{qid}");
    let out = w.command(&["answer", &id, "yes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    // ensure's spawn stamp: this revision started a worker for this server
    let stamp = w.lock().with_file_name(format!(
        "{}.spawned-{}",
        w.lock().file_name().unwrap().to_string_lossy(),
        herdr_inbox::store::revision::MIGRATION_REVISION
    ));
    assert!(stamp.exists(), "no worker was started");
    // that worker idles out by itself (IDLE_EXIT 1 s, inherited) and lets the lock go
    let deadline = Instant::now() + Duration::from_secs(60);
    while (is_running(&w.lock()) || stamp_fresh(&stamp)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!is_running(&w.lock()), "the started worker did not end");
    assert!(w.store.get(qid).unwrap().unwrap().verified_at.is_some());
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

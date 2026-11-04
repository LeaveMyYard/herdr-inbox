//! The Rust puller against the Rust remote side: `Store::pull` and
//! `Store::flush` (src/store/mirror) whose ssh runs this crate's `inbox
//! export` and `inbox apply` on another database. An answer queued on the
//! operator's machine lands on the remote once, also when its first reply
//! is lost.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::plugin_env::config::ConfigFile;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::cli::{Io, run};
use herdr_inbox::remote::command::{FixedCommand, SafeTarget};
use herdr_inbox::service::own::Me;
use herdr_inbox::store::Store;
use herdr_inbox::store::mirror::CallResult;
use serde_json::{Map, Value, json};

use crate::common::migrated;
use crate::fakes::{Effects, FakeDelivery, FakeHerdr, World};

/// The remote machine: its own database and stubbed herdr.
struct Remote {
    dir: tempfile::TempDir,
    effects: Effects,
}

impl Remote {
    fn new() -> Remote {
        let dir = tempfile::tempdir().unwrap();
        migrated(dir.path());
        Remote {
            dir,
            effects: Rc::default(),
        }
    }

    fn db(&self) -> PathBuf {
        self.dir.path().join("inbox.db")
    }

    fn store(&self) -> Store {
        Store::open(&self.db(), Duration::from_secs(5)).unwrap()
    }

    /// `inbox <cmd>` with `stdin`, as ssh would run it there.
    fn call(&self, cmd: &str, stdin: &str) -> CallResult {
        self.call_bytes(cmd, stdin.as_bytes())
    }

    fn call_bytes(&self, cmd: &str, stdin: &[u8]) -> CallResult {
        let world = Rc::new(RefCell::new(World::default()));
        let mut herdr = FakeHerdr {
            world: Rc::clone(&world),
            effects: Rc::clone(&self.effects),
        };
        let mut delivery = FakeDelivery {
            world,
            effects: Rc::clone(&self.effects),
        };
        let files = Transcripts::new(Some(ProjectsRoot::new(self.dir.path().join("projects"))));
        let mut config = ConfigFile::in_dir(self.dir.path().join("config"));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut input = stdin;
        let db = self.db();
        let code = run(
            cmd,
            &[],
            &mut Io {
                input: &mut input,
                out: &mut out,
                err: &mut err,
                db_path: &db,
                state_dir: self.dir.path(),
                files: &files,
                me: Me {
                    session: None,
                    pane: None,
                },
                delivery: &mut delivery,
                herdr: &mut herdr,
                config: &mut config,
                herdr_config: &self.dir.path().join("herdr.toml"),
                lock_wait: Duration::from_secs(1),
                now: 0,
            },
        );
        CallResult {
            code,
            stdout: String::from_utf8(out).unwrap(),
            stderr: String::from_utf8(err).unwrap(),
        }
    }
}

/// The operator's machine: a mirror of machine `m1`, its paths known (no probe).
fn operator(dir: &Path) -> Store {
    let store = Store::open(&migrated(dir), Duration::from_secs(5)).unwrap();
    store
        .record_machine("m1", Some("box"), Some("box"))
        .unwrap();
    store
        .db()
        .execute_batch(
            "UPDATE remote_machines SET uv = '/usr/bin/uv', herdr = '/usr/bin/herdr', \
             plugin_root = '/opt/inbox' WHERE machine_id = 'm1'",
        )
        .unwrap();
    store
}

/// ssh to the remote: the fixed `inbox export|apply` command lines only.
/// `lose` drops the reply of that many apply calls after they ran.
fn ssh<'a>(
    remote: &'a Remote,
    lose: &'a RefCell<usize>,
) -> impl FnMut(&SafeTarget, &FixedCommand, Option<&str>) -> CallResult + 'a {
    move |_, command, stdin| {
        let cmd = command.as_str().rsplit(' ').next().unwrap();
        assert!(
            command.as_str().ends_with(&format!(" inbox {cmd}")),
            "{command:?}"
        );
        let got = remote.call(cmd, stdin.unwrap_or_default());
        if cmd == "apply" && *lose.borrow() > 0 {
            *lose.borrow_mut() -= 1;
            return CallResult {
                code: 255,
                stdout: String::new(),
                stderr: "Connection reset".into(),
            };
        }
        got
    }
}

/// The mirrored question `qid` as the operator's popup shows it.
fn shown(store: &Store, qid: i64) -> Map<String, Value> {
    let (data, db_id): (String, String) = store
        .db()
        .query_row(
            "SELECT data_json, db_id FROM remote_questions WHERE remote_id = ?",
            [qid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let mut data: Map<String, Value> = serde_json::from_str(&data).unwrap();
    data.insert("_db_id".into(), db_id.into());
    data
}

#[test]
fn an_answer_queued_on_the_operators_machine_lands_on_the_remote_once() {
    let remote = Remote::new();
    remote
        .store()
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, pane_id, session_id, question, norm, status) \
             VALUES (1, '2001-01-01T00:00:00+00:00', 'w1:p1', 's', 'Merge?', 'merge?', 'open')",
        )
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = operator(dir.path());
    let lose = RefCell::new(0);
    let mut call = ssh(&remote, &lose);
    let (ok, notices) = mirror
        .pull("m1", Some("box"), Some("box"), SystemTime::now(), &mut call)
        .unwrap();
    assert!(
        ok,
        "{:?}",
        mirror
            .db()
            .query_row("SELECT last_error FROM remote_machines", [], |r| r
                .get::<_, Option<String>>(0))
    );
    assert_eq!(notices.len(), 1, "{notices:?}");
    let data = shown(&mirror, 1);
    let mut clock = SystemTime::now;
    let queued = mirror
        .enqueue(
            "m1",
            &data,
            "answer",
            Some(&json!({"text": "yes"})),
            "req-1",
            &mut clock,
            Duration::from_secs(3600),
        )
        .unwrap();
    assert_eq!(queued, Ok("req-1".to_string()));
    // the first reply is lost after the remote applied it: the retry is answered from the record
    *lose.borrow_mut() = 1;
    mirror.flush("m1", &mut clock, &mut call).unwrap();
    let q = remote.store().get(1).unwrap().unwrap();
    assert_eq!(
        (q.status.as_str(), q.answer.as_deref()),
        ("answered", Some("yes"))
    );
    let state = |m: &Store| -> String {
        m.db()
            .query_row(
                "SELECT state FROM remote_outbox WHERE request_id = 'req-1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        state(&mirror),
        "queued",
        "a lost reply leaves the request queued"
    );
    mirror
        .db()
        .execute("UPDATE remote_outbox SET next_try_at = NULL", [])
        .unwrap();
    mirror.flush("m1", &mut clock, &mut call).unwrap();
    assert_ne!(state(&mirror), "queued");
    let applied: Vec<(String, String)> = {
        let r = remote.store();
        let mut stmt = r
            .db()
            .prepare("SELECT request_id, outcome FROM applied_requests")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(applied, [("req-1".to_string(), "applied".to_string())]);
    let spawned: Vec<Value> = remote
        .effects
        .borrow()
        .iter()
        .filter(|e| e[0] == "spawn_deliver")
        .cloned()
        .collect();
    assert_eq!(
        spawned,
        [json!(["spawn_deliver", [1], false])],
        "delivered once"
    );
    // the next pull mirrors it answered
    assert!(
        mirror
            .pull("m1", Some("box"), Some("box"), SystemTime::now(), &mut call)
            .unwrap()
            .0
    );
    assert_eq!(shown(&mirror, 1)["status"], "answered");
}

#[test]
fn a_request_python_could_not_read_or_store_is_refused_unrecorded() {
    let remote = Remote::new();
    // a lone surrogate reads as U+FFFD here; Python's SQLite write raises on it
    let got = remote.call("apply", r#"{"request_id": "r\udc00", "action": "status"}"#);
    assert_eq!(
        (got.code, got.stdout.as_str()),
        (
            0,
            "{\"outcome\": \"rejected\", \"reason\": \"bad request\"}\n"
        )
    );
    let n: i64 = remote
        .store()
        .db()
        .query_row("SELECT COUNT(*) FROM applied_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    // bytes that are not UTF-8 (Python's stdin read raises)
    let world = remote.call("export", "{\"cursor\": \"\u{fffd}\"}");
    assert_eq!(world.code, 0, "U+FFFD itself is fine");
    let err = remote.call_bytes("export", b"{\"x\": \"\xff\"}");
    assert_eq!(err.code, 1);
    assert!(
        err.stdout.starts_with("{\"error\": \"bad request: "),
        "{}",
        err.stdout
    );
}

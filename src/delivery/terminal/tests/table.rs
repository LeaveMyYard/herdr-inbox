//! The core rows of the delivery decision table
//! (`tests/fixtures/decisions-core.json`): the real pass, store and transcripts
//! over a scripted `hc` and process table. `hc` is a stand-in by construction:
//! nothing here can reach a core.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use herdr_agent_kit::agent_procs::Pid;
use herdr_agent_kit::agent_procs::fake::FakeProcesses;
use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::plugin_env::LockFile;
use inbox_core::deliver::{MAX_STALE_RETRIES, Mode};
use serde_json::{Value, json};

use crate::claude::Transcripts;
use crate::delivery::run::{Ctx, Unreadable, deliver_batch};
use crate::delivery::terminal::{Hosted, Terminals};
use crate::host::core::Machine;
use crate::host::core::hc::{Hc, NO_TERMINAL, Sent};
use crate::host::core::record::Record;
use crate::store::hosts::{CORE_SERVER, CoreSeat};
use crate::store::liveness::{Stamp, touch};
use crate::store::questions::NewQuestion;
use crate::store::{Question, Store};
use crate::worker::duties::Round;
use crate::worker::duties::retry::retry_round;

const TABLE: &str = include_str!("../../../../tests/fixtures/decisions-core.json");

/// The older session, which continued into the asker.
const OLDER: &str = "00000001-0000-4000-8000-0000000000a1";
/// The asking session.
const ASKER: &str = "00000002-0000-4000-8000-0000000000a2";
/// The session the asker continued into.
const NEWER: &str = "00000003-0000-4000-8000-0000000000a3";
/// A conversation of its own.
const OTHER: &str = "00000004-0000-4000-8000-0000000000a4";

fn screen(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hc/read-0.1.18")
        .join(format!("{name}.json"));
    let read: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    read["text"].as_str().unwrap().to_owned()
}

/// `hc`, as one row of the table has it.
struct Hc1 {
    world: Value,
    transcript: PathBuf,
    typed: Mutex<Vec<String>>,
    tried: Mutex<usize>,
}

impl Hc for Hc1 {
    fn get(&self, address: &str) -> Result<Value, &'static str> {
        assert_eq!(address, "t3");
        match self.world["hc"].as_str().unwrap() {
            "gone" => return Err(NO_TERMINAL),
            "down" => return Err("no core is running"),
            _ => {}
        }
        let (pid, name, argv) = match self.world["foreground"].as_str().unwrap() {
            "claude" => (20, "claude", vec!["claude"]),
            "shell" => (19, "zsh", vec!["zsh"]),
            "tool" => (21, "cargo", vec!["cargo", "test"]),
            "nested" => (22, "claude", vec!["claude", "-p"]),
            other => panic!("{other}"),
        };
        let state = match self.world["hc"].as_str().unwrap() {
            "finished" => "finished",
            _ => "live",
        };
        Ok(json!({
            "address": "t3", "state": state, "status": self.world["status"],
            "foreground": {"pid": pid, "name": name, "argv": argv, "cwd": "/r"},
        }))
    }

    fn screen(&self, address: &str) -> Result<String, &'static str> {
        assert_eq!(address, "t3");
        self.world["screen"]
            .as_str()
            .map(screen)
            .ok_or("hc read failed")
    }

    fn send(&self, address: &str, text: &str) -> Sent {
        assert_eq!(address, "t3");
        *self.tried.lock().unwrap() += 1;
        match self.world["at_send"].as_str() {
            Some("blocked") => return Sent::Blocked,
            Some("not_idle") => return Sent::NotIdle,
            Some("gone") => return Sent::Gone,
            _ => {}
        }
        self.typed.lock().unwrap().push(text.to_owned());
        if self.world["lands"] == true {
            let line = json!({"type": "user", "sessionId": ASKER, "timestamp": "2099-01-01T00:00:00+00:00",
                              "message": {"role": "user", "content": text}});
            let old = fs::read_to_string(&self.transcript).unwrap_or_default();
            fs::write(&self.transcript, format!("{old}{line}\n")).unwrap();
        }
        Sent::Typed
    }
}

struct Machine1 {
    world: Value,
    /// Looks so far: the row's `other_conversation` turns up on the second.
    looks: Mutex<usize>,
}

impl Machine for Machine1 {
    type Table = FakeProcesses;
    fn table(&self) -> FakeProcesses {
        let mut t = FakeProcesses::default();
        t.insert(19, 1, "zsh", "s19", &["zsh"]);
        t.insert(20, 19, "claude", "s20", &["claude"]);
        t.insert(21, 20, "cargo", "s21", &["cargo", "test"]);
        t.insert(22, 20, "claude", "s22", &["claude", "-p"]);
        t
    }
    fn parent(&self) -> Pid {
        1
    }
    fn recorded_session(&self, _: Pid, _: &str) -> Record {
        let mut looks = self.looks.lock().unwrap();
        *looks += 1;
        if *looks > 1 && self.world["at_send"] == "other_conversation" {
            return Record::Names(OTHER.into());
        }
        match self.world["record"].as_str().unwrap() {
            "asker" => Record::Names(ASKER.into()),
            "older" => Record::Names(OLDER.into()),
            "newer" => Record::Names(NEWER.into()),
            "other" => Record::Names(OTHER.into()),
            "missing" => Record::Missing,
            "unusable" => Record::Unusable,
            other => panic!("{other}"),
        }
    }
    fn live(&self, _: &str) -> String {
        panic!("a terminal's conversation is never followed")
    }
    fn pane_foreground(&self, _: &str) -> Option<Vec<Pid>> {
        None
    }
}

/// A row's world on disk: the database, the transcripts, the core row.
struct World {
    dir: tempfile::TempDir,
    store: Store,
    files: Transcripts,
    herdr: Herdr,
    id: i64,
}

fn transcript(dir: &Path, session: &str) -> PathBuf {
    dir.join("projects/-r").join(format!("{session}.jsonl"))
}

impl World {
    fn new(row_world: &Value) -> World {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("projects/-r")).unwrap();
        let line = |s: &str| format!("{}\n", json!({"type": "user", "sessionId": s}));
        for s in [OLDER, ASKER, NEWER, OTHER] {
            fs::write(transcript(dir.path(), s), line(s)).unwrap();
        }
        let link = |from: &str, to: &str| {
            let at = "2020-01-01T00:00:00+00:00";
            let l = json!({"type": "continued-in", "sessionId": from, "continuedInSessionId": to, "timestamp": at});
            let path = transcript(dir.path(), from);
            let old = fs::read_to_string(&path).unwrap();
            fs::write(path, format!("{old}{l}\n")).unwrap();
        };
        link(OLDER, ASKER);
        link(ASKER, NEWER);
        let store =
            Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
        let q = store
            .insert(&NewQuestion {
                session_id: Some(ASKER),
                cwd: Some("/r"),
                question: "Merge it?",
                ..NewQuestion::default()
            })
            .unwrap();
        store
            .db()
            .execute(
                "UPDATE question_ext SET herdr_server = ? WHERE id = ?",
                (CORE_SERVER, q.id),
            )
            .unwrap();
        let seat = CoreSeat {
            terminal: "t3".into(),
            machine: None,
        };
        store.set_question_host(q.id, &seat).unwrap();
        store.set_answer(q.id, "yes", None).unwrap();
        let world = World {
            files: Transcripts::new(Some(ProjectsRoot::new(dir.path().join("projects")))),
            herdr: Herdr::new(ServerId::new(dir.path().join("no-herdr.sock")))
                .with_bin("/nonexistent/herdr-cli"),
            store,
            id: q.id,
            dir,
        };
        world.prepare(row_world);
        world
    }

    /// The row's stored state before the pass.
    fn prepare(&self, w: &Value) {
        let db = self.store.db();
        if let Some(secs) = w["pending_for"].as_i64() {
            let since = crate::clock::now_minus(Duration::from_secs(secs as u64));
            let error = "pending: busy: the agent is working";
            db.execute(
                "UPDATE questions SET delivery_error = ? WHERE id = ?",
                (error, self.id),
            )
            .unwrap();
            db.execute(
                "UPDATE question_ext SET pending_since = ? WHERE id = ?",
                (since, self.id),
            )
            .unwrap();
        }
        let db_path = self.dir.path().join("inbox.db");
        if w["hook_live"] == true {
            touch(&db_path, ASKER, Stamp::HookSeen, SystemTime::now());
        }
        if w["watched"] == true {
            touch(&db_path, ASKER, Stamp::WatchSeen, SystemTime::now());
        }
        if w["landed_before"] == true {
            let q = self.question();
            let text = crate::delivery::messages::delivery_messages(&[
                crate::delivery::segments::answered(&q),
            ])
            .remove(0)
            .text;
            let line = json!({"type": "user", "sessionId": ASKER, "timestamp": "2099-01-01T00:00:00+00:00",
                              "message": {"role": "user", "content": text}});
            let path = transcript(self.dir.path(), ASKER);
            let old = fs::read_to_string(&path).unwrap();
            fs::write(path, format!("{old}{line}\n")).unwrap();
        }
    }

    fn question(&self) -> Question {
        self.store.get(self.id).unwrap().unwrap()
    }

    fn hosted(&self, w: &Value) -> Hosted<Hc1, Machine1> {
        Hosted {
            hc: Hc1 {
                world: w.clone(),
                transcript: transcript(self.dir.path(), self.typed_into(w)),
                typed: Mutex::default(),
                tried: Mutex::default(),
            },
            machine: Machine1 {
                world: w.clone(),
                looks: Mutex::new(0),
            },
        }
    }

    fn typed_into(&self, w: &Value) -> &'static str {
        match w["record"].as_str().unwrap() {
            "newer" => NEWER,
            _ => ASKER,
        }
    }

    fn pass(&self, core: &dyn Terminals, mode: Mode) {
        let mut notify = |_: &str, _: &str| {};
        let mut unreadable = Unreadable::after(Duration::ZERO);
        let mut ctx = self.ctx(core, &mut notify, &mut unreadable);
        deliver_batch(&mut ctx, &[self.question()], mode, None).unwrap();
    }

    fn ctx<'a>(
        &'a self,
        core: &'a dyn Terminals,
        notify: &'a mut dyn FnMut(&str, &str),
        unreadable: &'a mut Unreadable,
    ) -> Ctx<'a> {
        Ctx {
            store: &self.store,
            herdr: &self.herdr,
            core,
            files: &self.files,
            state_dir: self.dir.path(),
            notify,
            unreadable,
            verify_wait: Duration::from_millis(50),
        }
    }

    /// The row as the table says it: what a reader of the database sees.
    fn state(&self) -> Value {
        let q = self.question();
        let attempts: i64 = self
            .store
            .db()
            .query_row(
                "SELECT delivery_attempts FROM questions WHERE id = ?",
                [self.id],
                |r| r.get(0),
            )
            .unwrap();
        let error = q.delivery_error.clone().unwrap_or_default();
        if let Some(reason) = error.strip_prefix("pending: ") {
            return json!({"state": "pending", "reason": reason, "attempts": attempts});
        }
        if q.delivered_at.is_some() {
            let verified: Option<String> = self
                .store
                .db()
                .query_row(
                    "SELECT verified_at FROM question_ext WHERE id = ?",
                    [self.id],
                    |r| r.get(0),
                )
                .unwrap();
            return json!({"state": "delivered", "verified": verified.is_some(),
                          "acknowledged": q.acknowledged_at.is_some(), "attempts": attempts});
        }
        json!({"state": "failed", "error": error, "attempts": attempts})
    }
}

fn rows() -> Vec<Value> {
    let table: Value = serde_json::from_str(TABLE).unwrap();
    table["rows"].as_array().unwrap().clone()
}

fn mode_of(row: &Value) -> Mode {
    Mode {
        retry: row["mode"]["retry"] == true,
        force: row["mode"]["force"] == true,
        verify_later: false,
    }
}

#[test]
fn every_core_row_of_the_decision_table_holds() {
    let mut wrong = BTreeMap::new();
    for row in rows() {
        let w = &row["world"];
        let world = World::new(w);
        let lock = (w["watched"] == true).then(|| {
            let dir = world.dir.path().join("rewake");
            fs::create_dir_all(&dir).unwrap();
            LockFile::new(dir.join(format!("{ASKER}.lock")))
                .try_exclusive()
                .unwrap()
                .unwrap()
        });
        let hosted = world.hosted(w);
        world.pass(&hosted, mode_of(&row));
        drop(lock);
        let got = json!({
            "after": world.state(),
            "typed": hosted.hc.typed.lock().unwrap().len(),
            "tried": *hosted.hc.tried.lock().unwrap(),
        });
        let want = json!({"after": row["after"], "typed": row["typed"], "tried": row["tried"]});
        if got != want {
            wrong.insert(row["name"].as_str().unwrap().to_owned(), (want, got));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn the_table_covers_what_the_plan_names() {
    let names: Vec<String> = rows()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_owned())
        .collect();
    for name in [
        "draft",
        "blocked",
        "idle",
        "working-within-grace",
        "working-past-grace",
        "terminal-gone",
        "resumed-older-conversation",
    ] {
        assert!(names.iter().any(|n| n == name), "{name}");
    }
}

#[test]
fn a_typed_answer_is_typed_once_however_many_passes_follow() {
    let w = rows().remove(0)["world"].clone();
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    for _ in 0..3 {
        world.pass(&hosted, Mode::default());
    }
    let mut notify = |_: &str, _: &str| {};
    let mut unreadable = Unreadable::after(Duration::ZERO);
    let mut ctx = world.ctx(&hosted, &mut notify, &mut unreadable);
    let mut round = Round {
        ctx: &mut ctx,
        server: CORE_SERVER,
    };
    assert_eq!(retry_round(&mut round).unwrap(), 0);
    assert_eq!(hosted.hc.typed.lock().unwrap().len(), 1);
}

#[test]
fn a_gone_terminal_runs_out_of_tries_and_is_marked_gone() {
    let w = rows()
        .into_iter()
        .find(|r| r["name"] == "terminal-gone")
        .unwrap()["world"]
        .clone();
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world
        .store
        .db()
        .execute_batch("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00'")
        .unwrap();
    let mut notify = |_: &str, _: &str| {};
    let mut unreadable = Unreadable::after(Duration::ZERO);
    let mut ctx = world.ctx(&hosted, &mut notify, &mut unreadable);
    let mut round = Round {
        ctx: &mut ctx,
        server: CORE_SERVER,
    };
    for _ in 0..MAX_STALE_RETRIES + 2 {
        retry_round(&mut round).unwrap();
    }
    assert_eq!(retry_round(&mut round).unwrap(), 0);
    let error = world.question().delivery_error.unwrap();
    assert!(error.starts_with("agent gone: "), "{error}");
}

#[test]
fn a_herdr_worker_never_lists_a_core_row() {
    let w = rows().remove(0)["world"].clone();
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world
        .store
        .db()
        .execute_batch("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00'")
        .unwrap();
    let mut notify = |_: &str, _: &str| {};
    let mut unreadable = Unreadable::after(Duration::ZERO);
    let mut ctx = world.ctx(&hosted, &mut notify, &mut unreadable);
    let mut round = Round {
        ctx: &mut ctx,
        server: "/srv/herdr.sock",
    };
    assert_eq!(retry_round(&mut round).unwrap(), 0);
    assert_eq!(*hosted.hc.tried.lock().unwrap(), 0);
    round.server = CORE_SERVER;
    assert_eq!(retry_round(&mut round).unwrap(), 0);
    assert_eq!(hosted.hc.typed.lock().unwrap().len(), 1);
}

#[test]
fn another_machines_terminal_is_left_untouched() {
    let w = rows().remove(0)["world"].clone();
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world
        .store
        .db()
        .execute_batch("UPDATE question_host SET machine = 'vps'")
        .unwrap();
    world.pass(&hosted, Mode::default());
    assert_eq!(*hosted.hc.tried.lock().unwrap(), 0);
    assert_eq!(world.question().delivery_error, None);
    assert_eq!(world.store.undelivered_core(&[]).unwrap().len(), 0);
}

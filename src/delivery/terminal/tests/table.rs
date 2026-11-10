//! The core rows of the delivery decision table
//! (`tests/fixtures/decisions-core.json`): the real pass, store and transcripts
//! over a scripted `hc` and process table. `hc` is a stand-in by construction:
//! nothing here can reach a core.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use herdr_agent_kit::agent_procs::Pid;
use herdr_agent_kit::agent_procs::fake::FakeProcesses;
use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_agent_kit::plugin_env::LockFile;
use inbox_core::deliver::{MAX_STALE_RETRIES, Mode};
use serde_json::{Value, json};

use crate::claude::Transcripts;
use crate::delivery::run::{Ctx, Unreadable, deliver_batch};
use crate::delivery::terminal::{Hosted, Seen, Terminals};
use crate::host::core::Machine;
use crate::host::core::hc::{Hc, NO_TERMINAL, Sent};
use crate::host::core::record::{Other, Record};
use crate::store::candidates::has_candidates;
use crate::store::hosts::{CORE_SERVER, CoreSeat};
use crate::store::liveness::{Stamp, touch};
use crate::store::questions::NewQuestion;
use crate::store::{Question, Store};
use crate::worker::duties::moved::revive_moved;
use crate::worker::duties::retry::retry_round;
use crate::worker::duties::served::CAPS;
use crate::worker::duties::{Memory, Round};

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
    reads: Mutex<usize>,
    sent_at: Mutex<Vec<String>>,
    looks: AtomicUsize,
}

impl Hc for Hc1 {
    fn get(&self, address: &str) -> Result<Value, &'static str> {
        let elsewhere = [("t4", 24), ("t5", 25), ("t6", 26)];
        if let Some((_, pid)) = elsewhere.iter().find(|(t, _)| *t == address) {
            return Ok(json!({
                "address": address, "state": "live", "status": "idle",
                "foreground": {"pid": pid, "name": "claude", "argv": ["claude"], "cwd": "/r"},
            }));
        }
        assert_eq!(address, "t3");
        match self.world["hc"].as_str().unwrap() {
            "gone" => return Err(NO_TERMINAL),
            "down" => return Err("no core is running"),
            _ => {}
        }
        let mut status = self.world["status"].clone();
        if self.looks.fetch_add(1, Ordering::Relaxed) > 0 && self.world["at_send"] == "started" {
            status = json!("working"); // a turn began after the first look
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
            "address": "t3", "state": state, "status": status,
            "foreground": {"pid": pid, "name": name, "argv": argv, "cwd": "/r"},
        }))
    }

    fn screen(&self, address: &str) -> Result<String, &'static str> {
        assert_eq!(address, "t3");
        let mut reads = self.reads.lock().unwrap();
        *reads += 1;
        // the panel, the box, then the last read before the send
        if *reads > 2 && self.world["at_send"] == "draft" {
            return Ok(screen("draft"));
        }
        self.world["screen"]
            .as_str()
            .map(screen)
            .ok_or("hc read failed")
    }

    fn list(&self) -> Vec<String> {
        let elsewhere = self.world["elsewhere"].as_str().unwrap_or_default();
        let others = elsewhere.split(',').filter(|t| !t.is_empty());
        std::iter::once("t3")
            .chain(others)
            .map(str::to_owned)
            .collect()
    }

    fn send(&self, address: &str, text: &str, status: &str) -> Sent {
        assert_eq!(address, "t3");
        *self.tried.lock().unwrap() += 1;
        self.sent_at.lock().unwrap().push(status.to_owned());
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
        t.insert(23, 1, "claude", "s23", &["claude"]);
        t.insert(24, 1, "claude", "s24", &["claude"]);
        t.insert(25, 1, "claude", "s25", &["claude"]);
        t.insert(26, 1, "claude", "s26", &["claude"]);
        t
    }
    fn parent(&self) -> Pid {
        1
    }
    fn recorded_session(&self, pid: Pid, _: &str) -> Record {
        match pid {
            24 | 25 => return Record::Names(ASKER.into()),
            26 => return Record::Names(OTHER.into()),
            _ => {}
        }
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
    fn other_records(&self, _: Pid) -> Vec<Other> {
        if self.world["twin"] == true {
            vec![Other::Runs(23, "s23".into(), ASKER.into())]
        } else {
            Vec::new()
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
        if let Some(secs) = w["answered_ago"].as_u64().filter(|s| *s > 0) {
            let at = crate::clock::now_minus(Duration::from_secs(secs));
            db.execute(
                "UPDATE questions SET answered_at = ? WHERE id = ?",
                (at, self.id),
            )
            .unwrap();
        }
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
                reads: Mutex::default(),
                sent_at: Mutex::default(),
                looks: AtomicUsize::new(0),
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
    world
        .store
        .db()
        .execute_batch("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00'")
        .unwrap();
    world.pass(&hosted, Mode::default());
    assert_eq!(*hosted.hc.tried.lock().unwrap(), 0);
    assert_eq!(world.question().delivery_error, None);
    assert_eq!(world.store.undelivered_core(&[]).unwrap().len(), 0);
    world
        .store
        .db()
        .execute_batch("UPDATE question_host SET machine = NULL")
        .unwrap();
    assert_eq!(
        world.store.undelivered_core(&[]).unwrap().len(),
        1,
        "aged past the grace"
    );
}

/// herdr lists the asker in a pane. A core row is not moved there: no worker
/// would serve it, its server being the core's. A hook there still carries it.
#[test]
fn a_core_row_is_never_moved_into_a_pane() {
    let row = rows()
        .into_iter()
        .find(|r| r["name"] == "terminal-gone")
        .unwrap();
    let w = &row["world"];
    let mut world = World::new(w);
    let fake = FakeHerdr::start();
    let agent = json!({"pane_id": "w1:p9", "tab_id": "w1:t9", "agent": "claude", "agent_status": "idle",
                       "agent_session": {"kind": "id", "value": ASKER}});
    fake.on("agent.list", Reply::Result(json!({"agents": [agent]})));
    world.herdr = fake.blocking_client().with_bin("/nonexistent/herdr-cli");
    let hosted = world.hosted(w);
    world.pass(&hosted, mode_of(&row));
    assert_eq!(world.question().pane_id, None);
    assert_eq!(world.state(), row["after"]);
}

/// herdr's stuck notice is a herdr worker's: a row of no server shows on the
/// core worker's tick too, and it must not use up that row's one notice.
#[test]
fn the_core_worker_raises_no_stuck_notice() {
    let w = rows().remove(0)["world"].clone();
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world
        .store
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, pane_id, session_id, question, norm, status, answer, \
             answered_at, delivery_error, delivery_attempts) VALUES (50, '2020-01-01T00:00:00+00:00', 'w1:p1', \
             'x', 'q', 'q', 'answered', 'a', '2020-01-01T00:00:00+00:00', 'no live agent in pane w1:p1', 10); \
             INSERT INTO question_ext (id, group_id) VALUES (50, 50);",
        )
        .unwrap();
    let notes = Mutex::new(0);
    let mut notify = |_: &str, _: &str| *notes.lock().unwrap() += 1;
    let mut unreadable = Unreadable::after(Duration::ZERO);
    let mut ctx = world.ctx(&hosted, &mut notify, &mut unreadable);
    let mut round = Round {
        ctx: &mut ctx,
        server: CORE_SERVER,
    };
    retry_round(&mut round).unwrap();
    assert_eq!(*notes.lock().unwrap(), 0);
    round.server = "/srv/herdr.sock";
    retry_round(&mut round).unwrap();
    assert_eq!(
        *notes.lock().unwrap(),
        1,
        "still the herdr worker's to raise"
    );
}

fn world_of(name: &str) -> (Value, World) {
    let row = rows().into_iter().find(|r| r["name"] == name).unwrap();
    let world = World::new(&row["world"]);
    (row, world)
}

#[test]
fn a_moved_row_takes_its_new_terminal_and_is_typed_there_once_proved() {
    let (row, world) = world_of("moved-to-another-terminal");
    let hosted = world.hosted(&row["world"]);
    world.pass(&hosted, mode_of(&row));
    let seat = world.store.core_seat(world.id).unwrap().unwrap();
    assert_eq!(seat.terminal, "t4");
    assert_eq!(world.question().pane_id, None);
}

#[test]
fn a_gone_core_row_stays_a_candidate_of_its_hook() {
    let (_, world) = world_of("terminal-gone");
    world.store.mark_agent_gone(world.id, "t3").unwrap();
    let db = world.dir.path().join("inbox.db");
    assert!(has_candidates(&db, ASKER, &world.files));
    let rows = world.store.hook_candidates(ASKER, 2, &world.files).unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn a_gone_pane_row_does_not() {
    let (_, world) = world_of("terminal-gone");
    world
        .store
        .db()
        .execute_batch("DELETE FROM question_host")
        .unwrap();
    world.store.mark_agent_gone(world.id, "w1:p1").unwrap();
    let db = world.dir.path().join("inbox.db");
    assert!(!has_candidates(&db, ASKER, &world.files));
}

#[test]
fn a_gone_row_comes_back_when_its_conversation_is_found() {
    let (row, world) = world_of("moved-to-another-terminal");
    world.store.mark_agent_gone(world.id, "t3").unwrap();
    let hosted = world.hosted(&row["world"]);
    let mut notify = |_: &str, _: &str| {};
    let mut unreadable = Unreadable::after(Duration::ZERO);
    let mut ctx = world.ctx(&hosted, &mut notify, &mut unreadable);
    let mut round = Round {
        ctx: &mut ctx,
        server: CORE_SERVER,
    };
    revive_moved(&mut round, &mut Memory::default()).unwrap();
    assert_eq!(
        world.store.core_seat(world.id).unwrap().unwrap().terminal,
        "t4"
    );
    let error = world.question().delivery_error.unwrap();
    assert!(error.starts_with("pending: moved"), "{error}");
}

#[test]
fn a_stranded_row_shows_as_undelivered_and_stops_costing_hc_runs() {
    let (row, world) = world_of("stranded-at-error");
    let hosted = world.hosted(&row["world"]);
    world.pass(&hosted, mode_of(&row));
    let shown = world.store.failed_deliveries().unwrap();
    assert_eq!(shown.len(), 1, "the popup and the feed list it");
    let listed = world.store.undelivered_core(&CAPS).unwrap();
    assert!(listed.is_empty(), "no worker retries it");
    let db = world.dir.path().join("inbox.db");
    assert!(has_candidates(&db, ASKER, &world.files), "a hook still can");
}

#[test]
fn a_session_of_another_config_directory_is_read_there() {
    let w = rows().remove(0)["world"].clone();
    let world = World::new(&w);
    let other = world.dir.path().join("elsewhere");
    let terminals = Hosted {
        hc: world.hosted(&w).hc,
        machine: Elsewhere(other.clone()),
    };
    assert!(matches!(terminals.see("t3", &[]), Seen::Unread(_)));
    let Seen::Here(seated) = terminals.see("t3", std::slice::from_ref(&other)) else {
        panic!("not read in that directory");
    };
    assert_eq!(seated.config.as_ref(), Some(&other));
    assert_eq!(seated.agent.session.as_deref(), Some(ASKER));
}

/// A machine whose Claude keeps its records in one other directory.
struct Elsewhere(PathBuf);

impl Machine for Elsewhere {
    type Table = FakeProcesses;
    fn table(&self) -> FakeProcesses {
        let mut t = FakeProcesses::default();
        t.insert(19, 1, "zsh", "s19", &["zsh"]);
        t.insert(20, 19, "claude", "s20", &["claude"]);
        t
    }
    fn parent(&self) -> Pid {
        1
    }
    fn recorded_session(&self, _: Pid, _: &str) -> Record {
        Record::Missing
    }
    fn recorded_in(&self, config: &Path, _: Pid, _: &str) -> Record {
        assert_eq!(config, self.0);
        Record::Names(ASKER.into())
    }
    fn other_records(&self, _: Pid) -> Vec<Other> {
        Vec::new()
    }
    fn live(&self, _: &str) -> String {
        panic!("never followed")
    }
    fn pane_foreground(&self, _: &str) -> Option<Vec<Pid>> {
        None
    }
}

#[test]
fn a_turn_seen_on_the_last_look_goes_back_to_if_idle() {
    let mut w = rows().remove(0)["world"].clone();
    w["status"] = json!("done");
    w["at_send"] = json!("started");
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world.pass(&hosted, Mode::default());
    assert_eq!(*hosted.hc.sent_at.lock().unwrap(), ["working"]);
}

/// The review of #146 round 2, L1: a long working stretch, then one bad read.
#[test]
fn one_unreadable_pass_after_a_long_working_stretch_does_not_strand() {
    let (row, world) = world_of("working-is-not-stranded");
    let hosted = world.hosted(&row["world"]);
    world.pass(&hosted, mode_of(&row));
    assert_eq!(
        world.store.pending_since(world.id).unwrap(),
        None,
        "a readable pass stops the clock"
    );
    let mut unread = row["world"].clone();
    unread["record"] = json!("missing");
    let hosted = world.hosted(&unread);
    world.pass(&hosted, mode_of(&row));
    let error = world.question().delivery_error.unwrap();
    assert!(
        error.starts_with("pending: waiting: cannot read"),
        "{error}"
    );
    assert!(
        world.store.pending_since(world.id).unwrap().is_some(),
        "its own clock starts"
    );
}

/// The wait that strands is one that ran on, pass after pass.
#[test]
fn a_wait_strands_only_when_it_ran_on_for_the_bound() {
    let (row, world) = world_of("stranded-unread");
    world
        .store
        .db()
        .execute_batch("UPDATE question_ext SET pending_since = NULL")
        .unwrap();
    let hosted = world.hosted(&row["world"]);
    world.pass(&hosted, mode_of(&row));
    let error = world.question().delivery_error.unwrap();
    assert!(
        error.starts_with("pending: "),
        "the first pass of a wait: {error}"
    );
}

/// A pass of the same wait keeps the clock it started with.
#[test]
fn a_wait_keeps_its_clock_from_pass_to_pass() {
    let (row, world) = world_of("stranded-unread");
    let started = crate::clock::now_minus(Duration::from_secs(120));
    world
        .store
        .db()
        .execute("UPDATE question_ext SET pending_since = ?", [&started])
        .unwrap();
    let hosted = world.hosted(&row["world"]);
    world.pass(&hosted, mode_of(&row));
    assert_eq!(world.store.pending_since(world.id).unwrap(), Some(started));
}

/// `error` is no turn with a Stop hook to come: its grace and its refusals are one
/// wait, on one clock, so it strands in the end.
#[test]
fn an_error_terminal_keeps_its_clock_through_the_grace() {
    let mut w = rows().remove(0)["world"].clone();
    w["status"] = json!("error");
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world.pass(&hosted, Mode::default());
    let error = world.question().delivery_error.unwrap();
    assert_eq!(error, "pending: busy: the agent is error");
    assert!(
        world.store.pending_since(world.id).unwrap().is_some(),
        "the clock runs"
    );
}

/// A busy pass while working sets the grace's clock, and must not leave it running:
/// a bad read after it starts a wait of its own.
#[test]
fn a_working_pass_leaves_no_clock_behind() {
    let mut w = rows().remove(0)["world"].clone();
    w["status"] = json!("working");
    let world = World::new(&w);
    let hosted = world.hosted(&w);
    world.pass(&hosted, Mode::default());
    let error = world.question().delivery_error.unwrap();
    assert_eq!(error, "pending: busy: the agent is working");
    assert_eq!(world.store.pending_since(world.id).unwrap(), None);
}

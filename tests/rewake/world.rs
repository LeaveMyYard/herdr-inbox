//! A watcher's world: database, transcripts, a scripted herdr, a fake clock.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_inbox::claude::Transcripts;
use herdr_inbox::hook::payload::Payload;
use herdr_inbox::rewake::{Clock, Watcher, run};
use rusqlite::{Connection, params};
use serde_json::json;

use crate::common::{A, answer, ask, migrated, open};

/// What runs after the n-th sleep, with the clock's time to move.
pub type OnSleep<'a> = Box<dyn FnMut(u32, &Cell<f64>) + 'a>;

/// Time that moves only when the watcher sleeps; `on_sleep(n)` runs after the n-th sleep.
pub struct FakeClock<'a> {
    pub t: Cell<f64>,
    pub sleeps: Cell<u32>,
    pub on_sleep: RefCell<OnSleep<'a>>,
}

impl<'a> FakeClock<'a> {
    pub fn new(on_sleep: impl FnMut(u32, &Cell<f64>) + 'a) -> FakeClock<'a> {
        FakeClock {
            t: Cell::new(0.0),
            sleeps: Cell::new(0),
            on_sleep: RefCell::new(Box::new(on_sleep)),
        }
    }
}

impl Clock for FakeClock<'_> {
    fn now(&self) -> f64 {
        self.t.get()
    }
    fn sleep(&self, s: f64) {
        self.t.set(self.t.get() + s);
        self.sleeps.set(self.sleeps.get() + 1);
        (self.on_sleep.borrow_mut())(self.sleeps.get(), &self.t);
    }
}

pub struct World {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub herdr: FakeHerdr,
}

impl World {
    pub fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = migrated(dir.path());
        std::fs::create_dir_all(dir.path().join("projects").join("-r")).unwrap();
        let w = World {
            dir,
            db,
            herdr: FakeHerdr::start(),
        };
        w.agent(Some(A), "idle");
        w
    }

    pub fn conn(&self) -> Connection {
        open(&self.db)
    }

    /// herdr's `agent.get` reply for the pane from now on.
    pub fn agent(&self, session: Option<&str>, status: &str) {
        let mut agent = json!({"agent": "claude", "agent_status": status, "pane_id": "w1:p1", "tab_id": "w1:t1"});
        if let Some(s) = session {
            agent["agent_session"] = json!({"kind": "id", "value": s});
        }
        self.herdr
            .on("agent.get", Reply::Result(json!({"agent": agent})));
    }

    /// An answered question nobody has delivered yet.
    pub fn pending_answer(&self) -> i64 {
        let db = self.conn();
        let id = ask(&db, A, "Which colour?");
        answer(&db, id, "amber");
        id
    }

    pub fn watcher<'c>(&self, clock: &'c dyn Clock, pane: Option<&str>) -> Watcher<'c> {
        Watcher {
            db: self.db.clone(),
            state_dir: self.dir.path().to_path_buf(),
            pane: pane.map(str::to_owned),
            herdr: self.herdr.blocking_client(),
            files: Transcripts::new(Some(ProjectsRoot::new(self.dir.path().join("projects")))),
            owner: None,
            clock,
        }
    }

    pub fn payload(&self) -> Payload {
        Payload {
            session_id: Some(A.into()),
            ..Payload::default()
        }
    }

    pub fn watch(&self, clock: &dyn Clock) -> Option<String> {
        run(&self.watcher(clock, Some("w1:p1")), &self.payload())
    }

    pub fn delivered(&self, id: i64) -> bool {
        let at: Option<String> = self
            .conn()
            .query_row(
                "SELECT delivered_at FROM questions WHERE id = ?",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        at.is_some()
    }

    pub fn agent_gets(&self) -> usize {
        self.herdr.params_of("agent.get").len()
    }
}

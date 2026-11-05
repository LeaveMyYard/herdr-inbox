//! The worker's duties against a scripted herdr socket and a temporary
//! database (Python tests/test_delivery_worker.py, test_agent_gone.py and
//! their `worker.tick` scenarios). Nothing here reaches the operator's
//! herdr, database or transcripts: every path is under the test's own
//! temporary directory.

mod alerts;
mod ancestor_replay;
#[path = "../store/common.rs"]
mod common;
mod failed;
mod fake;
mod gone;
#[path = "../delivery/herdr_stub.rs"]
mod herdr_stub;
mod hooks;
mod live;
mod moved;
mod process;
mod reconcile;
mod retry;

use std::path::PathBuf;
use std::time::Duration;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::delivery::run::{Ctx, Unreadable, deliver_batch};
use herdr_inbox::delivery::{REFERENCE_INDEX, expected_segments};
use herdr_inbox::store::Store;
use herdr_inbox::store::question::normalize_options;
use herdr_inbox::store::questions::NewQuestion;
use herdr_inbox::worker::duties::retry::retry_round;
use herdr_inbox::worker::duties::tick::{Tock, tick};
use herdr_inbox::worker::duties::unverified::reconcile_unverified;
use herdr_inbox::worker::duties::{Memory, Round};
use inbox_core::deliver::Mode;

pub use common::A;
use common::migrated;
use fake::Fake;

/// A second caller in another pane (Python's `B`, the same conversation).
pub const PANE_B: (&str, &str) = ("w1:p2", "w1:t2");

/// One worker's world.
pub struct World {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub projects: PathBuf,
    pub fake: Fake,
    pub store: Store,
    pub herdr: Herdr,
    pub files: Transcripts,
    /// The server this worker serves (default: the fake's socket).
    pub server: String,
    /// Every notification shown: (title, body).
    pub notes: Vec<(String, String)>,
    /// What the worker remembers between ticks.
    pub memory: Memory,
    unreadable: Unreadable,
    n: usize,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// A fresh database, the fake, and the asking session's transcript
    /// (Python's `live_session` fixture).
    pub fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = migrated(dir.path());
        let projects = dir.path().join("claude/projects");
        let transcript = projects.join("-fake-project").join(format!("{A}.jsonl"));
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        std::fs::write(
            &transcript,
            "{\"type\": \"user\", \"message\": {\"content\": \"hello\"}}\n",
        )
        .unwrap();
        let fake = Fake::start(dir.path(), projects.clone());
        let herdr = Herdr::new(ServerId::new(&fake.stub.socket)).with_bin("/nonexistent/herdr-cli");
        let server = fake.stub.socket.to_string_lossy().into_owned();
        World {
            store: Store::open(&db, Duration::from_secs(5)).unwrap(),
            files: Transcripts::new(Some(ProjectsRoot::new(projects.clone()))),
            dir,
            db,
            projects,
            fake,
            herdr,
            server,
            notes: Vec::new(),
            memory: Memory::default(),
            unreadable: Unreadable::after(Duration::ZERO),
            n: 0,
        }
    }

    /// The asking session's transcript.
    pub fn transcript(&self) -> PathBuf {
        self.projects
            .join("-fake-project")
            .join(format!("{A}.jsonl"))
    }

    /// A question asked from `pane` (Python `service.ask`), on `server`.
    pub fn ask_on(&mut self, (pane, tab): (&str, &str), server: Option<&str>) -> i64 {
        self.n += 1;
        let text = format!("Merge it {}?", self.n);
        let q = self
            .store
            .insert(&NewQuestion {
                pane_id: Some(pane),
                tab_id: Some(tab),
                tab_label: Some("tab-A"),
                session_id: Some(A),
                cwd: Some("/r"),
                question: &text,
                options: normalize_options(&serde_json::json!(["yes", "no"])).unwrap(),
                ..NewQuestion::default()
            })
            .unwrap();
        self.store.set_server(q.id, server).unwrap();
        q.id
    }

    /// A question from w1:p1 on this worker's server.
    pub fn ask(&mut self) -> i64 {
        let server = self.server.clone();
        self.ask_on(("w1:p1", "w1:t1"), Some(&server))
    }

    /// Answers `qid` with the agent `status` and delivers it at once
    /// (Python `service.answer`: a fresh send, verified in this process).
    pub fn answer(&mut self, qid: i64, status: &str) {
        self.fake.set(|k| k.status = status.into());
        self.store.set_answer(qid, "yes", None).unwrap();
        let q = self.store.get(qid).unwrap().unwrap();
        self.with_ctx(|ctx, _| deliver_batch(ctx, &[q], Mode::default(), None).unwrap());
    }

    /// Python's `pending_answer`: asked, answered while the agent is `status`.
    pub fn pending_answer(&mut self, status: &str) -> i64 {
        let qid = self.ask();
        self.answer(qid, status);
        qid
    }

    fn with_ctx<T>(&mut self, f: impl FnOnce(&mut Ctx<'_>, &str) -> T) -> T {
        let mut notes = Vec::new();
        let mut notify = |t: &str, b: &str| notes.push((t.to_string(), b.to_string()));
        let mut ctx = Ctx {
            store: &self.store,
            herdr: &self.herdr,
            files: &self.files,
            state_dir: self.dir.path(),
            notify: &mut notify,
            unreadable: &mut self.unreadable,
            verify_wait: Duration::from_millis(300),
        };
        let out = f(&mut ctx, &self.server);
        self.notes.extend(notes);
        out
    }

    /// One tick's retry half; the answers still pending.
    pub fn tick(&mut self) -> usize {
        self.with_ctx(|ctx, server| retry_round(&mut Round { ctx, server }).unwrap())
    }

    /// A whole tick as the worker runs it: the re-check its database open
    /// makes, then the tick (no blocked scan: herdr reports no agents to it).
    pub fn full_tick(&mut self) -> Tock {
        reconcile_unverified(&self.store, &self.files).unwrap();
        self.tick_alone()
    }

    /// A tick alone, without the re-check of a database open.
    pub fn tick_alone(&mut self) -> Tock {
        let mut memory = std::mem::take(&mut self.memory);
        let tock = self.with_ctx(|ctx, server| {
            tick(&mut Round { ctx, server }, &mut memory, &mut |_| 0).unwrap()
        });
        self.memory = memory;
        tock
    }

    /// Runs `f` with a round of this world.
    pub fn with_round<T>(&mut self, f: impl FnOnce(&mut Round<'_, '_>) -> T) -> T {
        self.with_ctx(|ctx, server| f(&mut Round { ctx, server }))
    }

    /// The hook verification pass alone.
    pub fn verify_hooks(&mut self) {
        self.with_ctx(|ctx, server| {
            herdr_inbox::worker::duties::hooks::verify_hook_deliveries(&mut Round { ctx, server })
                .unwrap()
        });
    }

    /// Appends `records` to `session`'s transcript (Python `write`); its path.
    pub fn write(&self, session: &str, records: &[serde_json::Value]) -> PathBuf {
        use std::io::Write;
        let path = self
            .projects
            .join("-fake-project")
            .join(format!("{session}.jsonl"));
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        for r in records {
            writeln!(f, "{r}").unwrap();
        }
        path
    }

    /// Records `session`'s transcript path (Python `store.record_session`).
    pub fn record_session(&self, session: &str) {
        let path = self
            .projects
            .join("-fake-project")
            .join(format!("{session}.jsonl"));
        self.sql(
            "INSERT OR REPLACE INTO sessions (session_id, transcript_path) VALUES (?, ?)",
            rusqlite::params![session, path.to_string_lossy()],
        );
    }

    /// The message a delivery of `ids` types, as herdr types it.
    pub fn batch_text(&self, ids: &[i64]) -> String {
        let qs: Vec<_> = ids
            .iter()
            .map(|i| self.store.get(*i).unwrap().unwrap())
            .collect();
        let answered: Vec<_> = qs
            .iter()
            .map(herdr_inbox::delivery::segments::answered)
            .collect();
        let text = herdr_agent_kit::internal::inbox_contract::delivery::delivery_text(&answered);
        herdr_agent_kit::herdr_api::clean(&text, 1_000_000_000)
    }

    /// The long-answer reference a delivery of `qid` types.
    pub fn reference_text(&self, qid: i64) -> String {
        let q = self.store.get(qid).unwrap().unwrap();
        expected_segments(&q).swap_remove(REFERENCE_INDEX)
    }

    /// The notifications whose title contains `text`.
    pub fn shown(&self, text: &str) -> Vec<&(String, String)> {
        self.notes
            .iter()
            .filter(|(t, _)| t.contains(text))
            .collect()
    }

    /// A raw statement on the database.
    pub fn sql(&self, sql: &str, args: impl rusqlite::Params) {
        self.store.db().execute(sql, args).unwrap();
    }
}

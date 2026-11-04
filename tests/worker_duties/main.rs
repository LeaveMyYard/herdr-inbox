//! The worker's duties against a scripted herdr socket and a temporary
//! database (Python tests/test_delivery_worker.py, test_agent_gone.py and
//! their `worker.tick` scenarios). Nothing here reaches the operator's
//! herdr, database or transcripts: every path is under the test's own
//! temporary directory.

#[path = "../store/common.rs"]
mod common;
mod fake;
mod gone;
#[path = "../delivery/herdr_stub.rs"]
mod herdr_stub;
mod retry;

use std::path::PathBuf;
use std::time::Duration;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::delivery::run::{Ctx, Unreadable, deliver_batch};
use herdr_inbox::store::Store;
use herdr_inbox::store::question::normalize_options;
use herdr_inbox::store::questions::NewQuestion;
use herdr_inbox::worker::duties::Round;
use herdr_inbox::worker::duties::retry::retry_round;
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
        let herdr = Herdr::new(ServerId::new(&fake.stub.socket));
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

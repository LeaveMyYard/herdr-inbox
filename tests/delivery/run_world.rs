//! A one-pane world for the delivery-pass edge tests: an idle (or working)
//! Claude in w1:p1 running A, whose prompts are accepted and never land, its
//! answers in a fresh database, and a spy that snapshots every answer's row
//! the moment each prompt reaches herdr.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::delivery::run::{Ctx, Unreadable, deliver_batch};
use herdr_inbox::store::Store;
use inbox_core::deliver::Mode;
use rusqlite::params;
use serde_json::{Value, json};

use crate::common::{migrated, open};
use crate::herdr_stub::Stub;

pub const A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const EMPTY: &str = include_str!("../fixtures/composer/empty.ansi");

/// A pane and its answers.
pub struct World {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub ids: Vec<i64>,
    /// herdr's `agent_status` for the pane.
    pub status: &'static str,
    pub mode: Mode,
    /// How long a typed message is waited for.
    pub wait: Duration,
    /// The pane has no agent; A runs in this pane now.
    pub elsewhere: Option<&'static str>,
    /// What `pane.read` shows (default: an empty input box).
    pub screen: &'static str,
    /// The sessions table is lost when the first prompt reaches herdr.
    pub lose_sessions_at_prompt: bool,
    /// No verify-later callback, whatever `mode` says.
    pub no_callback: bool,
}

/// What a pass left: its result, herdr's requests, the verify-later jobs
/// (each with the number of prompts typed when it was handed over), and per
/// prompt the answers' rows as they stood when it reached herdr.
pub struct Ran {
    pub result: rusqlite::Result<()>,
    pub stub: Stub,
    pub jobs: Vec<(Value, usize)>,
    pub at_prompt: Vec<Vec<Value>>,
    /// The notifications shown.
    pub notes: Vec<Value>,
}

impl Ran {
    pub fn prompts(&self) -> usize {
        self.stub.params_of("agent.prompt").len()
    }
}

/// The rows of `ids`: (delivered, delivery_error with a claim as `<claim>`).
pub fn rows(db: &Path, ids: &[i64]) -> Vec<Value> {
    let conn = open(db);
    ids.iter()
        .map(|id| {
            conn.query_row(
                "SELECT delivered_at, delivery_error FROM questions WHERE id = ?",
                [id],
                |r| {
                    let error: Option<String> = r.get(1)?;
                    let error = error.map(|e| {
                        if e.starts_with("sending:") {
                            "<claim>".to_string()
                        } else {
                            e
                        }
                    });
                    Ok(json!([r.get::<_, Option<String>>(0)?.is_some(), error]))
                },
            )
            .unwrap()
        })
        .collect()
}

impl World {
    /// An idle pane whose answers are `answers`, in a retry pass.
    pub fn new(answers: &[&str]) -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = migrated(dir.path());
        let conn = open(&db);
        let ids = answers
            .iter()
            .enumerate()
            .map(|(i, answer)| {
                conn.execute(
                    "INSERT INTO questions (created_at, pane_id, session_id, question, norm, status, answer, answered_at) \
                     VALUES ('2026-10-21T10:00:00+00:00', 'w1:p1', ?, ?, ?, 'answered', ?, '2026-10-21T10:01:00+00:00')",
                    params![A, format!("Q{i}?"), format!("q{i}"), answer],
                )
                .unwrap();
                conn.last_insert_rowid()
            })
            .collect();
        World {
            dir,
            db,
            ids,
            status: "idle",
            mode: Mode {
                retry: true,
                ..Mode::default()
            },
            wait: Duration::from_millis(300),
            elsewhere: None,
            screen: EMPTY,
            lose_sessions_at_prompt: false,
            no_callback: false,
        }
    }

    /// One pass over the answers; `before` runs on the opened store first.
    pub fn pass(&self, before: impl FnOnce(&Store)) -> Ran {
        let dir = self.dir.path();
        let projects = dir.join("projects");
        std::fs::create_dir_all(projects.join("-r")).unwrap();
        std::fs::write(
            projects.join("-r").join(format!("{A}.jsonl")),
            "{\"type\": \"user\"}\n",
        )
        .unwrap();
        let at_prompt = Arc::new(Mutex::new(Vec::new()));
        let (spy, db, ids, status, elsewhere, screen, lose) = (
            Arc::clone(&at_prompt),
            self.db.clone(),
            self.ids.clone(),
            self.status,
            self.elsewhere,
            self.screen,
            self.lose_sessions_at_prompt,
        );
        let stub = Stub::start(dir, move |method, _| {
            Ok(match method {
                "agent.get" if elsewhere.is_some() => json!({}),
                "agent.list" => {
                    let there = elsewhere.map(|p| {
                        json!({"pane_id": p, "tab_id": "w1:t2", "agent": "claude",
                        "agent_status": "idle", "agent_session": {"kind": "id", "value": A}})
                    });
                    json!({"agents": there.into_iter().collect::<Vec<_>>()})
                }
                "agent.get" => {
                    json!({"agent": {"pane_id": "w1:p1", "agent": "claude", "agent_status": status,
                                                "agent_session": {"kind": "id", "value": A}}})
                }
                "pane.read" => json!({"read": {"text": screen}}),
                "agent.prompt" => {
                    spy.lock().unwrap().push(rows(&db, &ids));
                    if lose {
                        let _ =
                            open(&db).execute_batch("ALTER TABLE sessions RENAME TO sessions_gone");
                    }
                    json!({})
                }
                _ => json!({}),
            })
        });
        let client = Herdr::new(ServerId::new(&stub.socket)).with_bin("/nonexistent/herdr-cli");
        let store = Store::open(&self.db, Duration::from_secs(5)).unwrap();
        let files = Transcripts::new(Some(ProjectsRoot::new(projects)));
        let qs: Vec<_> = self
            .ids
            .iter()
            .map(|id| store.get(*id).unwrap().unwrap())
            .collect();
        before(&store);
        let mut notes = Vec::new();
        let mut notify = |title: &str, body: &str| notes.push(json!([title, body]));
        let mut unreadable = Unreadable::after(Duration::ZERO);
        let mut ctx = Ctx {
            store: &store,
            herdr: &client,
            core: &herdr_inbox::delivery::terminal::NoCore,
            files: &files,
            state_dir: dir,
            notify: &mut notify,
            unreadable: &mut unreadable,
            verify_wait: self.wait,
        };
        let mut jobs = Vec::new();
        let requests = Arc::clone(&stub.requests);
        let mut later = |job: Value| {
            let typed = requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "agent.prompt")
                .count();
            jobs.push((job, typed));
        };
        let job: Option<&mut dyn FnMut(Value)> = if self.mode.verify_later && !self.no_callback {
            Some(&mut later)
        } else {
            None
        };
        let result = deliver_batch(&mut ctx, &qs, self.mode, job);
        let at_prompt = at_prompt.lock().unwrap().clone();
        Ran {
            result,
            stub,
            jobs,
            at_prompt,
            notes,
        }
    }
}

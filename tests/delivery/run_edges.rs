//! What the Python scenarios cannot show: a database that fails mid-pass
//! (nothing is typed, the error comes back), and the one wait per typed
//! message however many answers it carries.

use std::path::Path;
use std::time::{Duration, Instant};

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

const A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const EMPTY: &str = include_str!("../fixtures/composer/empty.ansi");

/// An idle Claude in w1:p1 running A; prompts are accepted and never land.
fn idle(method: &str) -> Result<Value, String> {
    Ok(match method {
        "agent.get" => {
            json!({"agent": {"pane_id": "w1:p1", "agent": "claude", "agent_status": "idle",
                                        "agent_session": {"kind": "id", "value": A}}})
        }
        "pane.read" => json!({"read": {"text": EMPTY}}),
        _ => json!({}),
    })
}

fn answers(db: &Path, n: usize) -> Vec<i64> {
    let conn = open(db);
    (0..n)
        .map(|i| {
            conn.execute(
                "INSERT INTO questions (created_at, pane_id, session_id, question, norm, status, answer, answered_at) \
                 VALUES ('2026-10-21T10:00:00+00:00', 'w1:p1', ?, ?, ?, 'answered', 'yes', '2026-10-21T10:01:00+00:00')",
                params![A, format!("Q{i}?"), format!("q{i}")],
            )
            .unwrap();
            conn.last_insert_rowid()
        })
        .collect()
}

fn pass(dir: &Path, db: &Path, ids: &[i64], wait: Duration) -> (rusqlite::Result<()>, Stub) {
    let projects = dir.join("projects");
    std::fs::create_dir_all(projects.join("-r")).unwrap();
    std::fs::write(
        projects.join("-r").join(format!("{A}.jsonl")),
        "{\"type\": \"user\"}\n",
    )
    .unwrap();
    let stub = Stub::start(dir, |m, _| idle(m));
    let client = Herdr::new(ServerId::new(&stub.socket));
    let store = Store::open(db, Duration::from_secs(5)).unwrap();
    let files = Transcripts::new(Some(ProjectsRoot::new(projects)));
    let (mut notify, mut unreadable) = (|_: &str, _: &str| {}, Unreadable::default());
    let mut ctx = Ctx {
        store: &store,
        herdr: &client,
        files: &files,
        state_dir: dir,
        notify: &mut notify,
        unreadable: &mut unreadable,
        verify_wait: wait,
    };
    let qs: Vec<_> = ids
        .iter()
        .map(|id| store.get(*id).unwrap().unwrap())
        .collect();
    let result = deliver_batch(
        &mut ctx,
        &qs,
        Mode {
            retry: true,
            ..Mode::default()
        },
        None,
    );
    (result, stub)
}

#[test]
fn a_database_that_fails_mid_pass_types_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated(dir.path());
    let ids = answers(&db, 1);
    // the claim (an UPDATE of delivery_error) fails
    open(&db)
        .execute_batch(
            "CREATE TRIGGER no_claims BEFORE UPDATE OF delivery_error ON questions \
             BEGIN SELECT RAISE(ABORT, 'disk on fire'); END",
        )
        .unwrap();
    let (result, stub) = pass(dir.path(), &db, &ids, Duration::from_secs(1));
    assert!(result.unwrap_err().to_string().contains("disk on fire"));
    assert!(stub.params_of("agent.prompt").is_empty());
}

#[test]
fn a_message_is_waited_for_once_whatever_it_carries() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated(dir.path());
    let ids = answers(&db, 3);
    let started = Instant::now();
    let (result, stub) = pass(dir.path(), &db, &ids, Duration::from_millis(800));
    result.unwrap();
    let took = started.elapsed();
    assert_eq!(stub.params_of("agent.prompt").len(), 1); // the three in one message
    assert!(
        took < Duration::from_millis(1600),
        "waited per answer: {took:?}"
    );
}

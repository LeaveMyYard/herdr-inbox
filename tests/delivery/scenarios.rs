//! deliver_batch against Python's own runs of the same scenarios
//! (tests/fixtures/delivery-messages.json "scenarios", captured by
//! `tools/capture_goldens.py --only delivery`): a pane's world (questions,
//! transcripts, herdr's agent and screen, whether a typed prompt lands and
//! where), and what each question became, what was typed, what the operator
//! was told and the verify-later jobs. Here herdr is the kit's FakeHerdr and
//! a thread plays Claude, writing each typed prompt into a transcript.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{MAX_ARG, clean};
use herdr_agent_kit::internal::inbox_contract::delivery::delivery_text;
use herdr_agent_kit::plugin_env::LockFile;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::clock::now_minus;
use herdr_inbox::delivery::run::{Ctx, Unreadable, deliver_batch};
use herdr_inbox::delivery::segments::answered;
use herdr_inbox::pyjson::dumps;
use herdr_inbox::store::Store;
use inbox_core::deliver::Mode;
use rusqlite::params;
use serde_json::{Value, json};

use crate::common::{migrated, open};
use crate::herdr_stub::{Answer, Stub};

const GOLDEN: &str = include_str!("../fixtures/delivery-messages.json");
const A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn append(path: &Path, content: &str) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    // Python's json.dumps spacing: the files' sizes are the verify-later jobs' offsets
    let record = json!({"type": "user", "message": {"role": "user", "content": content}});
    writeln!(f, "{}", dumps(&record)).unwrap();
}

/// herdr's answer to one request of the scenario (the capture's stub, request for
/// request); a typed prompt lands in `land` before the reply, as there.
fn answer(sc: &Value, land: Option<&Path>, method: &str, params: &Value) -> Answer {
    match method {
        "agent.get" if sc["agent"].is_null() => Ok(json!({})),
        "agent.get" => {
            let mut rec = sc["agent"].clone();
            if let Some(reported) = sc.get("reported").and_then(Value::as_str) {
                // herdr reports this session; the agent runs the one it continued in
                rec["agent_session"] = json!({"kind": "id", "value": reported});
            }
            Ok(json!({"agent": rec}))
        }
        "agent.list" => Ok(json!({"agents": sc["agents"]})),
        "pane.read" => Ok(json!({"read": {"text": sc["screen"]}})),
        "tab.get" => Ok(
            json!({"tab": {"tab_id": params["tab_id"], "label": sc.get("tab").cloned().unwrap_or(json!("❓ node"))}}),
        ),
        "agent.prompt" if sc["prompt"] == "fail" => Err("fake failure".into()),
        "agent.prompt" => {
            if let Some(path) = land {
                append(
                    path,
                    &clean(params["text"].as_str().unwrap_or_default(), MAX_ARG),
                );
            }
            Ok(json!({}))
        }
        _ => Ok(json!({})),
    }
}

/// The questions as Python wrote them (asked 2 min ago, answered 1 min ago).
fn insert(db: &Path, sc: &Value) -> Vec<i64> {
    let conn = open(db);
    let mut ids = Vec::new();
    for (i, q) in sc["questions"].as_array().unwrap().iter().enumerate() {
        conn.execute(
            "INSERT INTO questions (created_at, pane_id, tab_id, tab_label, session_id, question, norm, status, answer, \
             answered_at) VALUES (?, 'w1:p1', 'w1:t1', ?, ?, ?, ?, 'answered', ?, ?)",
            params![now_minus(Duration::from_secs(120)), q["tab_label"].as_str(), q["session"].as_str(),
                    q["question"].as_str(), format!("n{i}"), q["answer"].as_str(), now_minus(Duration::from_secs(60))],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        if let Some(ago) = q.get("pending_ago").and_then(Value::as_u64) {
            conn.execute(
                "INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?)",
                params![id, id, now_minus(Duration::from_secs(ago))],
            )
            .unwrap();
        }
        ids.push(id);
    }
    ids
}

fn states(db: &Path, ids: &[i64]) -> Value {
    let conn = open(db);
    let rows: Vec<Value> = ids
        .iter()
        .map(|id| {
            conn.query_row(
                "SELECT q.status, q.pane_id, q.delivered_at, q.delivery_error, q.acknowledged_at, q.delivery_attempts, \
                 e.verified_at, e.busy_attempts, e.pending_since, q.tab_label FROM questions q \
                 LEFT JOIN question_ext e ON e.id = q.id \
                 WHERE q.id = ?",
                [id],
                |r| {
                    let error: Option<String> = r.get::<_, Option<String>>(3)?.filter(|e| !e.is_empty());
                    let error = error.map(|e| if e.starts_with("sending:") { "<claim>".to_string() } else { e });
                    Ok(json!({"status": r.get::<_, String>(0)?, "pane": r.get::<_, Option<String>>(1)?,
                              "delivered": r.get::<_, Option<String>>(2)?.is_some_and(|s| !s.is_empty()),
                              "error": error,
                              "acknowledged": r.get::<_, Option<String>>(4)?.is_some_and(|s| !s.is_empty()),
                              "attempts": r.get::<_, i64>(5)?,
                              "verified": r.get::<_, Option<String>>(6)?.is_some_and(|s| !s.is_empty()),
                              "busy_attempts": r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                              "pending_since": r.get::<_, Option<String>>(8)?.is_some_and(|s| !s.is_empty()),
                              "tab_label": r.get::<_, Option<String>>(9)?}))
                },
            )
            .unwrap()
        })
        .collect();
    Value::Array(rows)
}

/// A job as the capture normalizes it: paths below the projects directory,
/// claims as `<claim>`.
fn normalized(mut job: Value, projects: &Path) -> Value {
    let root = format!("{}/", projects.display());
    let strip = |v: &mut Value| *v = json!(v.as_str().unwrap().replace(&root, ""));
    strip(&mut job["path"]);
    for w in job["watch"].as_array_mut().unwrap() {
        strip(&mut w[0]);
    }
    for row in job["rows"].as_array_mut().unwrap() {
        row["claim"] = if row["claim"].is_null() {
            Value::Null
        } else {
            json!("<claim>")
        };
        for c in row["counting"].as_array_mut().unwrap() {
            strip(c);
        }
    }
    job
}

fn run(sc: &Value) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("claude").join("projects");
    let folder = projects.join("-fake-project");
    fs::create_dir_all(&folder).unwrap();
    for (session, records) in sc["transcripts"].as_object().unwrap() {
        let text: String = records
            .as_array()
            .unwrap()
            .iter()
            .map(|r| format!("{}\n", dumps(r)))
            .collect();
        fs::write(folder.join(format!("{session}.jsonl")), text).unwrap();
    }
    let db = migrated(dir.path());
    let ids = insert(&db, sc);
    let store = Store::open(&db, Duration::from_secs(5)).unwrap();
    if sc.get("watch_stamp").is_some_and(|w| w == true) {
        open(&db)
            .execute(
                "INSERT INTO sessions (session_id, watch_seen_at) VALUES (?, ?)",
                params![A, now_minus(Duration::ZERO)],
            )
            .unwrap();
    }
    // a live rewake watcher holds its lock (another open file: flock sees it as another holder)
    let lock_file = dir.path().join("rewake").join(format!("{A}.lock"));
    let _watcher = sc.get("watch_lock").is_some_and(|w| w == true).then(|| {
        fs::create_dir_all(lock_file.parent().unwrap()).unwrap();
        LockFile::new(&lock_file).exclusive().unwrap()
    });
    for (id, q) in ids.iter().zip(sc["questions"].as_array().unwrap()) {
        if let Some(older) = q.get("older_landed").and_then(Value::as_str) {
            let stored = store.get(*id).unwrap().unwrap(); // typed there by an earlier pass
            append(
                &folder.join(format!("{older}.jsonl")),
                &delivery_text(&[answered(&stored)]),
            );
        }
        if q.get("already").is_some_and(|a| a == true) {
            let stored = store.get(*id).unwrap().unwrap();
            append(
                &folder.join(format!("{}.jsonl", q["session"].as_str().unwrap())),
                &delivery_text(&[answered(&stored)]),
            );
        }
    }
    let land: Option<PathBuf> = (sc["prompt"] == "land")
        .then(|| folder.join(format!("{}.jsonl", sc["land_in"].as_str().unwrap())));
    let scenario = sc.clone();
    let stub = Stub::start(dir.path(), move |method, params| {
        answer(&scenario, land.as_deref(), method, params)
    });
    let client = Herdr::new(ServerId::new(&stub.socket));
    let files = Transcripts::new(Some(ProjectsRoot::new(projects.clone())));
    let (mut notes, mut jobs) = (Vec::<Value>::new(), Vec::<Value>::new());
    {
        let mut notify = |title: &str, body: &str| notes.push(json!([title, body]));
        let mut unreadable = Unreadable::after(Duration::ZERO);
        let mut ctx = Ctx {
            store: &store,
            herdr: &client,
            files: &files,
            state_dir: dir.path(),
            notify: &mut notify,
            unreadable: &mut unreadable,
            verify_wait: Duration::from_secs(1),
        };
        let m = &sc["mode"];
        let mode = Mode {
            retry: m["retry"].as_bool().unwrap(),
            force: m["force"].as_bool().unwrap(),
            verify_later: m["verify_later"].as_bool().unwrap(),
        };
        let mut later = |job: Value| jobs.push(normalized(job, &projects));
        for _ in 0..sc.get("passes").and_then(Value::as_u64).unwrap_or(1) {
            let qs: Vec<_> = ids
                .iter()
                .map(|id| store.get(*id).unwrap().unwrap())
                .collect();
            let job: Option<&mut dyn FnMut(Value)> = if mode.verify_later {
                Some(&mut later)
            } else {
                None
            };
            deliver_batch(&mut ctx, &qs, mode, job).unwrap();
        }
    }
    let prompts: Vec<Value> = stub
        .params_of("agent.prompt")
        .iter()
        .map(|p| p["text"].clone())
        .collect();
    json!({"states": states(&db, &ids), "prompts": prompts, "notifications": notes, "jobs": jobs})
}

#[test]
fn every_scenario_ends_as_it_does_in_python() {
    let doc: Value = serde_json::from_str(GOLDEN).unwrap();
    let scenarios = doc["scenarios"].as_array().unwrap();
    assert!(scenarios.len() >= 36);
    for row in scenarios {
        let sc = &row["scenario"];
        let mut got = run(sc);
        if sc["prompt"] == "fail" {
            // the error's wording is the socket client's, not the CLI's: it carries herdr's message
            for state in got["states"].as_array_mut().unwrap() {
                let e = state["error"].as_str().unwrap_or_default();
                assert!(e.contains("fake failure"), "{e}");
                state["error"] = json!("fake failure");
            }
        }
        let want = json!({"states": row["states"], "prompts": row["prompts"], "notifications": row["notifications"],
                          "jobs": row["jobs"]});
        assert_eq!(got, want, "{}", row["name"]);
    }
}

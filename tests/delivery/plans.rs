//! Where a typed answer counts, against Python's answers over the same
//! world (tests/fixtures/delivery-messages.json "plans", captured by
//! `tools/capture_goldens.py --only delivery`): a conversation that continued
//! OLD -> NEW, an unrelated session, and questions of each.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::delivery::plans::{
    older_conversation, question_family, verify_plans, watch_paths,
};
use herdr_inbox::store::{Question, Store};
use inbox_core::deliver::Agent;
use rusqlite::params;
use serde_json::{Value, json};

use crate::common::{migrated, open};

const GOLDEN: &str = include_str!("../fixtures/delivery-messages.json");

struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Store,
    files: Transcripts,
    questions: Vec<Question>,
}

fn world(plans: &Value) -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("projects");
    for (rel, content) in plans["files"].as_object().unwrap() {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content.as_str().unwrap()).unwrap();
    }
    let db = migrated(dir.path());
    let conn = open(&db);
    for (i, q) in plans["questions"].as_array().unwrap().iter().enumerate() {
        conn.execute(
            "INSERT INTO questions (id, created_at, pane_id, session_id, question, norm, status, answer) \
             VALUES (?, ?, 'w1:p1', ?, ?, ?, 'answered', 'yes')",
            params![q["id"].as_i64(), q["created_at"].as_str(), q["session_id"].as_str(), format!("Q{i}?"), format!("q{i}")],
        )
        .unwrap();
    }
    let store = Store::open(&db, std::time::Duration::from_secs(5)).unwrap();
    let questions = plans["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| store.get(q["id"].as_i64().unwrap()).unwrap().unwrap())
        .collect();
    let files = Transcripts::new(Some(ProjectsRoot::new(root.clone())));
    World {
        _dir: dir,
        root,
        store,
        files,
        questions,
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap().to_string_lossy().into_owned()
}

#[test]
fn every_family_older_conversation_watch_and_plan_is_pythons() {
    let doc: Value = serde_json::from_str(GOLDEN).unwrap();
    let plans = &doc["plans"];
    let w = world(plans);
    let db = w.store.db();
    for (q, want) in w.questions.iter().zip(plans["family"].as_array().unwrap()) {
        assert_eq!(
            json!(question_family(db, &w.files, q).unwrap()),
            want["family"],
            "family of {}",
            q.id
        );
    }
    for row in plans["older"].as_array().unwrap() {
        let got =
            older_conversation(db, &w.files, row["live"].as_str(), row["asker"].as_str()).unwrap();
        assert_eq!(got, row["older"].as_bool().unwrap(), "{row}");
    }
    for row in plans["watch_paths"].as_array().unwrap() {
        let agent = Agent {
            kind: Some("claude".into()),
            status: "idle".into(),
            focused: false,
            session: row["agent"]["session_id"].as_str().map(String::from),
            reported_session: row["agent"]["reported_session_id"]
                .as_str()
                .map(String::from),
        };
        let got: Vec<Value> = watch_paths(db, &w.files, &agent, row["fresh"].as_bool().unwrap())
            .unwrap()
            .iter()
            .map(|(p, off)| json!([rel(&w.root, p), off]))
            .collect();
        assert_eq!(Value::Array(got), row["paths"], "{row}");
    }
    let vp = &plans["verify_plans"];
    let watch: Vec<(PathBuf, u64)> = vp["watch"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w2| {
            (
                w.root.join(w2[0].as_str().unwrap()),
                w2[1].as_u64().unwrap(),
            )
        })
        .collect();
    let batch: Vec<&Question> = w.questions.iter().collect();
    let got: BTreeMap<String, Value> =
        verify_plans(db, &w.files, vp["pane"].as_str().unwrap(), &watch, &batch)
            .unwrap()
            .into_iter()
            .map(|(id, plan)| {
                let counting: Vec<String> = plan.counting.iter().map(|p| rel(&w.root, p)).collect();
                (
                    id.to_string(),
                    json!({"counting": counting, "older_error": plan.older_error}),
                )
            })
            .collect();
    assert_eq!(json!(got), vp["plans"]);
}

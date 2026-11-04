use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use super::*;
use crate::mcp::args::Item;
use crate::mcp::caller::Pane;
use crate::mcp::effects::Herald;
use crate::mcp::receipts;
use crate::store::Store;

/// herdr with pane w1:p1 in tab w1:t1 ("my-tab"), session S.
struct OnePane;

impl World for OnePane {
    fn nested(&self) -> bool {
        false
    }
    fn pane(&self, pane: &str) -> Option<Pane> {
        (pane == "w1:p1").then(|| Pane {
            tab_id: Some("w1:t1".into()),
            session: Some("S".into()),
        })
    }
    fn tab_label(&self, _: &str) -> Option<String> {
        Some("my-tab".into())
    }
    fn agents(&self) -> Vec<(String, Option<String>)> {
        Vec::new()
    }
    fn live(&self, reported: &str, _: Option<&str>) -> String {
        reported.to_owned()
    }
    fn marks(&self, _: Option<&str>) -> Result<Vec<String>, Failure> {
        Ok(vec!["❓".into()])
    }
}

#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Herald for Log {
    fn notify(&self, title: &str, _: &str) {
        self.0.lock().unwrap().push(title.to_owned());
    }
    fn markers(&self, pane: &str) {
        self.0.lock().unwrap().push(format!("markers {pane}"));
    }
}

static STARTS: AtomicUsize = AtomicUsize::new(0);

fn runtime(db: PathBuf, log: Arc<Log>) -> Runtime {
    Runtime {
        env: Env {
            pane_id: Some("w1:p1".into()),
            ..Env::default()
        },
        world: Box::new(OnePane),
        files: Transcripts::new(None),
        db,
        server: "/sock".into(),
        backoff: Backoff {
            busy: vec![Duration::ZERO; 3],
            not_ready: vec![Duration::ZERO; 5],
        },
        effects: Arc::new(Effects::start(log)),
        ensure_worker: Box::new(|| {
            STARTS.fetch_add(1, Ordering::SeqCst);
        }),
        sleep: |_| {},
    }
}

fn ask(question: &str) -> Call {
    Call::AskOperator(Ask {
        question: Some(question.into()),
        ..Ask::default()
    })
}

#[test]
fn a_database_not_migrated_yet_gets_the_unavailable_receipt_and_asks_for_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let before = STARTS.load(Ordering::SeqCst);
    // no file at all: the worker creates and migrates it
    let rt = runtime(dir.path().join("inbox.db"), Arc::default());
    let all = [
        ask("Too early?"),
        Call::CheckAnswers,
        Call::ListMyQuestions {
            include_closed: false,
        },
        Call::WithdrawQuestion {
            id: 1,
            reason: "x".into(),
        },
        Call::AlertOperator {
            title: "Look".into(),
            message: None,
            severity: "warning".into(),
            asked_by: None,
        },
        Call::WithdrawAlert { id: 1 },
    ];
    for call in all {
        assert_eq!(rt.call(call), Ok(receipts::not_migrated()));
    }
    // an empty database: not migrated either, and nothing is written to it
    std::fs::write(dir.path().join("empty.db"), b"").unwrap();
    let rt = runtime(dir.path().join("empty.db"), Arc::default());
    assert_eq!(rt.call(ask("Too early?")), Ok(receipts::not_migrated()));
    assert_eq!(STARTS.load(Ordering::SeqCst) - before, 7 * 6); // every attempt of every call (#50)
    assert!(!dir.path().join("inbox.db").exists());
    assert_eq!(
        std::fs::metadata(dir.path().join("empty.db"))
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn a_question_is_filed_its_effects_queued_and_its_answer_checked_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    Store::open_migrating(&db, Duration::from_secs(3)).unwrap();
    let log = Arc::new(Log::default());
    let rt = runtime(db.clone(), log.clone());
    let r = rt.call(ask("Deploy?")).unwrap();
    assert_eq!(
        (r["id"].clone(), r["status"].clone()),
        (json!(1), json!("open"))
    );
    let group = rt
        .call(Call::AskOperator(Ask {
            questions: Some(vec![Item {
                question: "Region?".into(),
                header: None,
                options: None,
                multi_select: false,
            }]),
            ..Ask::default()
        }))
        .unwrap();
    assert_eq!(group["group"], json!(2)); // `questions` gets the group's receipt
    assert!(rt.effects.drain(Duration::from_secs(5)));
    assert_eq!(
        *log.0.lock().unwrap(),
        [
            "Q1 · my-tab",
            "markers w1:p1",
            "Q2 · my-tab",
            "markers w1:p1"
        ]
    );
    let store = Store::open(&db, Duration::from_secs(3)).unwrap();
    store.set_answer(1, "yes", None).unwrap();
    assert_eq!(
        rt.call(Call::CheckAnswers),
        Ok(json!({"answers": [{"id": 1, "question": "Deploy?", "answer": "yes"}]}))
    );
    assert_eq!(rt.call(Call::CheckAnswers), Ok(json!({"answers": []})));
}

#[test]
fn a_refused_ask_never_opens_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path().join("inbox.db"), Arc::default());
    let r = rt.call(Call::AskOperator(Ask::default())).unwrap();
    assert_eq!(r["status"], json!("error"));
}

#[test]
fn a_pre_bridge_stamp_with_the_whole_schema_works_and_is_never_restamped() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    Store::open_migrating(&db, Duration::from_secs(3)).unwrap();
    // an older Python MCP server re-stamped its crc32 (tests/fixtures/migrate.json `current_pre_bridge`)
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/migrate.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let stamp = fixture["current_pre_bridge"].as_i64().unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", stamp).unwrap();
    let schema = |c: &rusqlite::Connection| -> Vec<String> {
        let mut st = c
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let before = schema(&conn);
    let rt = runtime(db.clone(), Arc::default());
    assert_eq!(
        rt.call(ask("Stale stamp?")).unwrap()["status"],
        json!("open")
    );
    let alert = Call::AlertOperator {
        title: "Look".into(),
        message: None,
        severity: "warning".into(),
        asked_by: None,
    };
    assert_eq!(rt.call(alert).unwrap()["status"], json!("open"));
    let listed = rt
        .call(Call::ListMyQuestions {
            include_closed: false,
        })
        .unwrap();
    assert_eq!(listed["questions"][0]["question"], json!("Stale stamp?"));
    let now: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(now, stamp);
    assert_eq!(schema(&conn), before);
}

/// One pane whose agent's session can change between calls (`/clear`).
struct Clearable(Arc<Mutex<String>>);

impl World for Clearable {
    fn nested(&self) -> bool {
        false
    }
    fn pane(&self, _: &str) -> Option<Pane> {
        Some(Pane {
            tab_id: Some("w1:t1".into()),
            session: Some(self.0.lock().unwrap().clone()),
        })
    }
    fn tab_label(&self, _: &str) -> Option<String> {
        None
    }
    fn agents(&self) -> Vec<(String, Option<String>)> {
        Vec::new()
    }
    fn live(&self, reported: &str, _: Option<&str>) -> String {
        reported.to_owned()
    }
    fn marks(&self, _: Option<&str>) -> Result<Vec<String>, Failure> {
        Ok(Vec::new())
    }
}

/// Python `test_session_change_is_resolved_per_call`: the caller is resolved on every call.
#[test]
fn the_session_is_resolved_on_every_call() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    Store::open_migrating(&db, Duration::from_secs(3)).unwrap();
    let session = Arc::new(Mutex::new("aaaa".to_owned()));
    let mut rt = runtime(db, Arc::default());
    rt.world = Box::new(Clearable(Arc::clone(&session)));
    assert_eq!(rt.call(ask("Which bucket?")).unwrap()["id"], json!(1));
    assert_eq!(
        rt.call(ask("Which bucket?")).unwrap()["status"],
        json!("already_open")
    );
    *session.lock().unwrap() = "bbbb".into(); // after /clear: a new conversation, not a repeat
    let r = rt.call(ask("Which bucket?")).unwrap();
    assert_eq!(
        (r["id"].clone(), r["status"].clone()),
        (json!(2), json!("open"))
    );
}

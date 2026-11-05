//! Python's `fake_herdr` (tests/conftest.py) on a socket: one pane per id,
//! its agent's status, session and focus set by the test between ticks
//! (`FAKE_HERDR_STATUS`, `FAKE_HERDR_AGENT`, `FAKE_HERDR_SESSION`, ...). A
//! typed prompt lands in the session's transcript like a real Claude pane
//! (`FAKE_HERDR_LAND`), and every request is recorded.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use herdr_agent_kit::herdr_api::{MAX_ARG, clean};
use herdr_inbox::pyjson::dumps;
use serde_json::{Value, json};

use crate::herdr_stub::Stub;

pub const EMPTY: &str = include_str!("../fixtures/composer/empty.ansi");

/// The fake's knobs (Python's `FAKE_HERDR_*` variables).
#[derive(Clone, Debug)]
pub struct Knobs {
    /// `FAKE_HERDR_STATUS`.
    pub status: String,
    /// `FAKE_HERDR_AGENT=1`: the pane has an agent.
    pub agent: bool,
    /// `FAKE_HERDR_SESSION`: the session herdr reports.
    pub session: Option<String>,
    /// `FAKE_HERDR_FOCUSED`.
    pub focused: bool,
    /// `FAKE_HERDR_LAND`: `user`, `queue` or `none`.
    pub land: &'static str,
    /// `FAKE_HERDR_SCREEN`: the plain screen (the panel check).
    pub screen: String,
    /// `FAKE_HERDR_AGENTS`: `agent list`.
    pub agents: Value,
    /// `agent list`'s whole result, instead of `{"agents": agents}`.
    pub agent_list_result: Option<Value>,
    /// `tab get` labels (`FAKE_HERDR_TABS`, default `my-tab`).
    pub tab_label: String,
    /// `FAKE_HERDR_FAIL=agent list`.
    pub fail_agent_list: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs {
            status: "idle".into(),
            agent: true,
            session: Some(crate::A.into()),
            focused: false,
            land: "user",
            screen: String::new(),
            agents: json!([]),
            agent_list_result: None,
            tab_label: "my-tab".into(),
            fail_agent_list: false,
        }
    }
}

/// The running fake.
pub struct Fake {
    pub stub: Stub,
    pub knobs: Arc<Mutex<Knobs>>,
}

fn append(path: &PathBuf, record: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(f, "{}", dumps(record)).unwrap();
}

impl Fake {
    /// Serves in `dir`; transcripts land under `projects/-fake-project`.
    pub fn start(dir: &std::path::Path, projects: PathBuf) -> Fake {
        let knobs = Arc::new(Mutex::new(Knobs::default()));
        let k = Arc::clone(&knobs);
        let stub = Stub::start(dir, move |method, params| {
            let k = k.lock().unwrap().clone();
            let target = params["target"]
                .as_str()
                .or(params["pane_id"].as_str())
                .unwrap_or("w1:p1")
                .to_string();
            Ok(match method {
                "agent.get" if !k.agent => return Err("agent_not_found".into()),
                "agent.get" => {
                    let mut agent = json!({"pane_id": target, "agent": "claude",
                                           "agent_status": k.status, "focused": k.focused});
                    if let Some(s) = &k.session {
                        agent["agent_session"] = json!({"agent": "claude", "kind": "id", "source": "herdr:claude", "value": s});
                    }
                    json!({"agent": agent})
                }
                "agent.prompt" => {
                    let text = clean(params["text"].as_str().unwrap_or_default(), MAX_ARG);
                    if let (Some(s), false) = (&k.session, k.land == "none") {
                        let record = if k.land == "queue" {
                            json!({"type": "queue-operation", "operation": "enqueue", "content": text})
                        } else {
                            json!({"type": "user", "message": {"role": "user", "content": text}})
                        };
                        append(
                            &projects.join("-fake-project").join(format!("{s}.jsonl")),
                            &record,
                        );
                    }
                    json!({})
                }
                "pane.read" if params["format"] == "ansi" => json!({"read": {"text": EMPTY}}),
                "pane.read" => json!({"read": {"text": k.screen}}),
                "agent.list" if k.fail_agent_list => return Err("fake failure".into()),
                "agent.list" => k
                    .agent_list_result
                    .unwrap_or_else(|| json!({"agents": k.agents})),
                "tab.get" => json!({"tab": {"tab_id": params["tab_id"], "label": k.tab_label}}),
                "pane.list" => json!({"panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1"},
                                                 {"pane_id": "w1:p2", "tab_id": "w1:t2"}]}),
                _ => json!({}),
            })
        });
        Fake { stub, knobs }
    }

    /// Changes the knobs.
    pub fn set(&self, f: impl FnOnce(&mut Knobs)) {
        f(&mut self.knobs.lock().unwrap());
    }

    /// The panes prompts were typed into.
    pub fn prompts(&self) -> Vec<String> {
        self.stub
            .params_of("agent.prompt")
            .iter()
            .map(|p| {
                p["pane_id"]
                    .as_str()
                    .or(p["target"].as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    /// Forgets the requests so far (Python `fake_herdr.reset()`).
    pub fn reset(&self) {
        self.stub.requests.lock().unwrap().clear();
    }

    /// How many `agent.get` calls were made.
    pub fn agent_gets(&self) -> usize {
        self.stub.params_of("agent.get").len()
    }
}

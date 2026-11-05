//! The CLI replays tests/fixtures/cli.json (tools/capture_cli.py): Python's
//! `cli.main` over a script of command lines, with herdr, the delivery and
//! the worker stubbed at their function boundary. Every step must give the
//! same exit code, stdout, stderr and effects, and leave the same rows.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::plugin_env::config::ConfigFile;
use herdr_inbox::claude::Transcripts;
use herdr_inbox::cli::{Io, run};
use herdr_inbox::deeplink::lock::hold_popup_lock;
use herdr_inbox::delivery::send::local_answers;
use herdr_inbox::service::answer::submit;
use herdr_inbox::service::own::Me;
use herdr_inbox::store::Store;
use herdr_inbox::store::sessions::record_session;
use herdr_inbox::surface::markers::Markers;
use herdr_inbox::tabs::settings;
use serde_json::{Value, json};

use crate::common::migrated;
use crate::deeplink;
use crate::fakes::{Effects, FakeDelivery, FakeHerdr, World};
use crate::radar;
use crate::remote;
use crate::store_golden::{s, sql, tables};

const GOLDEN: &str = include_str!("../fixtures/cli.json");
const TABLES: [&str; 3] = ["questions", "question_ext", "tab_decor"];

struct Rig {
    dir: tempfile::TempDir,
    config: ConfigFile,
    files: Transcripts,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("plugin-config")).unwrap();
        std::fs::create_dir_all(dir.path().join("claude/projects/-r")).unwrap();
        let config = ConfigFile::in_dir(dir.path().join("plugin-config"));
        let files = Transcripts::new(Some(ProjectsRoot::new(dir.path().join("claude/projects"))));
        Rig { dir, config, files }
    }

    fn db(&self) -> std::path::PathBuf {
        self.dir.path().join("inbox.db")
    }

    fn transcript(&self, sid: &str) -> std::path::PathBuf {
        self.dir
            .path()
            .join(format!("claude/projects/-r/{sid}.jsonl"))
    }

    /// Writes the step's config and transcripts, as the capture does.
    fn prepare(&self, a: &Value) {
        if let Some(text) = s(a, "config") {
            std::fs::write(self.dir.path().join("plugin-config/config.toml"), text).unwrap();
        }
        if let Some(files) = a.get("files").and_then(Value::as_object) {
            for (sid, records) in files {
                let body: String = records
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| format!("{}\n", herdr_inbox::pyjson::dumps(r)))
                    .collect();
                std::fs::write(self.transcript(sid), body).unwrap();
            }
        }
    }

    /// The masking the capture applies to stderr.
    fn masked(&self, err: &[u8]) -> String {
        let text = String::from_utf8(err.to_vec()).unwrap();
        let text = text.replace(&self.dir.path().display().to_string(), "<work>");
        text.lines()
            .map(|l| match l.find("config.toml: ") {
                Some(i) => format!("{}<reason>\n", &l[..i + "config.toml: ".len()]),
                None => format!("{l}\n"),
            })
            .collect()
    }
}

fn now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    i64::try_from(secs).unwrap()
}

/// A `cli` step: (code, stdout as printed, stderr masked).
fn cli(
    rig: &mut Rig,
    a: &Value,
    world: &Rc<RefCell<World>>,
    effects: &Effects,
) -> (i32, String, String) {
    let db = rig.db();
    if let Some(list) = a.get("sessions").and_then(Value::as_array) {
        let store = Store::open(&db, Duration::from_secs(5)).unwrap();
        for sid in list {
            let sid = sid.as_str().unwrap();
            let path = rig.transcript(sid).display().to_string();
            record_session(store.db(), sid, Some(&path)).unwrap();
        }
    }
    let argv: Vec<String> = a["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let me = a.get("me").cloned().unwrap_or(Value::Null);
    let mut herdr = FakeHerdr {
        world: Rc::clone(world),
        effects: Rc::clone(effects),
    };
    let mut delivery = FakeDelivery {
        world: Rc::clone(world),
        effects: Rc::clone(effects),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input = s(a, "stdin").unwrap_or_default().as_bytes();
    let state_dir = rig.dir.path().to_path_buf();
    let herdr_config = rig.dir.path().join("herdr-config/config.toml");
    let code = {
        let mut io = Io {
            input: &mut input,
            out: &mut out,
            err: &mut err,
            db_path: &db,
            state_dir: &state_dir,
            files: &rig.files,
            me: Me {
                session: s(&me, "session"),
                pane: s(&me, "pane"),
            },
            delivery: &mut delivery,
            herdr: &mut herdr,
            config: &mut rig.config,
            herdr_config: &herdr_config,
            lock_wait: Duration::from_secs(1),
            now: now(),
        };
        run(&argv[0], &argv[1..], &mut io)
    };
    let out = String::from_utf8(out).unwrap();
    let out = out.replace(&rig.dir.path().display().to_string(), "<work>");
    (code, out, rig.masked(&err))
}

fn submit_step(rig: &mut Rig, a: &Value, world: &Rc<RefCell<World>>, effects: &Effects) -> Value {
    let store = Store::open(&rig.db(), Duration::from_secs(5)).unwrap();
    let qids: Vec<i64> = a["qids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let mut herdr = FakeHerdr {
        world: Rc::clone(world),
        effects: Rc::clone(effects),
    };
    let db = rig.db();
    let mut warn = |m: &str| panic!("warned: {m}");
    let mut markers = Markers {
        store: &store,
        db_path: &db,
        herdr: &mut herdr,
        config: &mut rig.config,
        lock_wait: Duration::from_secs(1),
        warn: &mut warn,
    };
    let done = submit(&store, &qids, None, &mut markers).unwrap();
    Value::Array(done.iter().map(|q| json!([q.id, q.answer])).collect())
}

fn preamble(db: &Path, a: &Value, effects: &Effects) -> Value {
    let store = Store::open(db, Duration::from_secs(5)).unwrap();
    let qs: Vec<_> = a["qids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| store.get(v.as_i64().unwrap()).unwrap().unwrap())
        .collect();
    let retry = a["retry"].as_bool().unwrap();
    let mut ensure = |server: Option<&str>| effects.borrow_mut().push(json!(["ensure", server]));
    let kept = local_answers(&store, &qs, retry, s(a, "server").unwrap(), &mut ensure).unwrap();
    Value::Array(kept.iter().map(|q| json!(q.id)).collect())
}

#[test]
fn the_cli_does_what_pythons_did() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let mut rig = Rig::new();
    let mut migrated_yet = false;
    let mut fresh = false; // the remote side's database, its own tables
    let mut last = Value::Null; // the tables as the golden last dumped them
    for (n, step) in golden["steps"].as_array().unwrap().iter().enumerate() {
        let (op, a) = (step["op"].as_str().unwrap(), &step["args"]);
        let effects: Effects = Rc::default();
        let world = Rc::new(RefCell::new(World::from_step(a)));
        rig.prepare(a);
        let what = format!("step {n} {op} {a}");
        match op {
            "migrate" => {
                migrated(rig.dir.path());
                migrated_yet = true;
            }
            "fresh" => {
                remote::fresh(rig.dir.path());
                fresh = true;
            }
            "sql" => sql(&Store::open(&rig.db(), Duration::from_secs(5)).unwrap(), a),
            "submit" => {
                let got = submit_step(&mut rig, a, &world, &effects);
                assert_eq!(got, step["result"], "{what}");
            }
            "preamble" => {
                let got = preamble(&rig.db(), a, &effects);
                assert_eq!(got, step["result"], "{what}");
            }
            "deeplink" => {
                let got = deeplink::step(&rig.db(), rig.dir.path(), a, &effects);
                assert_eq!(got, step["result"], "{what}");
            }
            "radar_missing" => {
                world.borrow_mut().radar_dir = radar::prepare(rig.dir.path(), a);
                let hint = settings::read(&mut rig.config, &mut |_| {}).radar_hint;
                let path = rig.dir.path().join("herdr-config/config.toml");
                let mut herdr = FakeHerdr {
                    world: Rc::clone(&world),
                    effects: Rc::clone(&effects),
                };
                let got = herdr_inbox::radar::missing(&path, hint, &mut herdr, &mut |_| {});
                assert_eq!(json!(got), step["result"], "{what}");
            }
            "radar" | "sync" | "cli" => {
                if op == "radar" {
                    world.borrow_mut().radar_dir = radar::prepare(rig.dir.path(), a);
                }
                let held = (a.get("popup_open") == Some(&json!(true)))
                    .then(|| hold_popup_lock(rig.dir.path()).unwrap());
                let (code, printed, err) = cli(&mut rig, a, &world, &effects);
                let remote_cmd = ["export", "apply"].contains(&a["argv"][0].as_str().unwrap());
                let out = if remote_cmd {
                    remote::mask(&printed)
                } else {
                    printed.clone()
                };
                drop(held);
                let left = deeplink::left_request(rig.dir.path());
                assert_eq!(
                    left,
                    step.get("request").cloned().unwrap_or(Value::Null),
                    "{what}: request"
                );
                assert_eq!(code, step["code"].as_i64().unwrap() as i32, "{what}: code");
                assert_eq!(out, step["stdout"].as_str().unwrap(), "{what}: stdout");
                assert_eq!(err, step["stderr"].as_str().unwrap(), "{what}: stderr");
                if a["argv"] == json!(["export"]) && code == 0 {
                    remote::cross_check(&printed, step["stdout"].as_str().unwrap(), &what);
                }
                if op == "radar" {
                    let (text, backup) = radar::left(rig.dir.path());
                    assert_eq!(text, step["herdr_config"], "{what}: herdr's config");
                    assert_eq!(json!(backup), step["backup"], "{what}: backup");
                }
            }
            other => panic!("unknown op {other}"),
        }
        assert_eq!(
            Value::Array(effects.borrow().clone()),
            step["effects"],
            "{what}: effects"
        );
        if migrated_yet {
            let store = Store::open(&rig.db(), Duration::from_secs(5)).unwrap();
            if step["tables"] != "unchanged" {
                last = step["tables"].clone();
            }
            let got = if fresh {
                remote::tables(&store)
            } else {
                tables(&store, &TABLES)
            };
            assert_eq!(got, last, "{what}: tables");
        } else {
            assert_eq!(step["tables"], Value::Null, "{what}");
            assert!(!rig.db().exists(), "{what}: the CLI created the database");
        }
    }
}

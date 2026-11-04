//! The `herdr-inbox` binary's `list`, `answer` and `dismiss` end to end: a
//! real database, herdr a scripted socket that plays Claude too (a typed
//! prompt lands in the asking session's transcript), the environment
//! cleared to the test's own (Python tests/test_delivery.py
//! `test_cli_answer_list_dismiss`).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use herdr_agent_kit::herdr_api::{MAX_ARG, clean};
use herdr_inbox::pyjson::dumps;
use serde_json::{Value, json};

use crate::common::{migrated, open};
use crate::herdr_stub::Stub;

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const EMPTY: &str = include_str!("../fixtures/composer/empty.ansi");

struct Rig {
    dir: tempfile::TempDir,
    db: PathBuf,
    stub: Stub,
}

fn append(path: &Path, content: &str) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    let record = json!({"type": "user", "message": {"role": "user", "content": content}});
    writeln!(f, "{}", dumps(&record)).unwrap();
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let db = migrated(dir.path());
        let project = dir.path().join("claude/projects/-r");
        fs::create_dir_all(&project).unwrap();
        let transcript = project.join(format!("{S}.jsonl"));
        fs::write(&transcript, "{\"type\": \"user\"}\n").unwrap();
        fs::create_dir_all(dir.path().join("cfg")).unwrap();
        fs::write(dir.path().join("cfg/config.toml"), "tab_prefix = \"❓\"\n").unwrap();
        let stub = Stub::start(dir.path(), move |method, params| {
            Ok(match method {
                "agent.get" => {
                    json!({"agent": {"pane_id": "w1:p1", "agent": "claude", "agent_status": "idle",
                                                "agent_session": {"kind": "id", "value": S}}})
                }
                "agent.list" => json!({"agents": []}),
                "pane.read" => json!({"read": {"text": EMPTY}}),
                "pane.list" => json!({"panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1"}]}),
                "tab.get" => json!({"tab": {"tab_id": "w1:t1", "label": "❓2 my-tab"}}),
                "plugin.list" => json!({"plugins": [
                    {"plugin_id": "someone.else", "plugin_root": "/elsewhere"},
                    {"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "/inbox/root"}]}),
                "agent.prompt" => {
                    append(
                        &transcript,
                        &clean(params["text"].as_str().unwrap_or_default(), MAX_ARG),
                    );
                    json!({})
                }
                _ => json!({}),
            })
        });
        let conn = open(&db);
        for q in ["one?", "two?"] {
            conn.execute(
                "INSERT INTO questions (created_at, pane_id, tab_id, tab_label, session_id, cwd, question, norm, status) \
                 VALUES (strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now', '-2 minutes'), 'w1:p1', 'w1:t1', 'my-tab', ?, '/r', ?, ?, 'open')",
                rusqlite::params![S, q, q],
            )
            .unwrap();
        }
        Rig { dir, db, stub }
    }

    /// `herdr-inbox <args>` with only the test's own environment.
    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    /// [`Rig::run`] with `extra` variables too.
    fn run_with(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let d = self.dir.path();
        Command::new(env!("CARGO_BIN_EXE_herdr-inbox"))
            .args(args)
            .env_clear()
            .envs(extra.iter().copied())
            .env("HOME", d)
            .env("PATH", "/usr/bin:/bin")
            .env("XDG_STATE_HOME", d.join("state"))
            .env("XDG_CONFIG_HOME", d.join("xdg"))
            .env("HERDR_SOCKET_PATH", &self.stub.socket)
            .env("HERDR_BIN_PATH", d.join("no-herdr"))
            .env("HERDR_INBOX_DB", &self.db)
            .env("HERDR_PLUGIN_CONFIG_DIR", d.join("cfg"))
            .env("CLAUDE_CONFIG_DIR", d.join("claude"))
            .output()
            .unwrap()
    }

    fn status(&self, id: i64) -> String {
        open(&self.db)
            .query_row("SELECT status FROM questions WHERE id = ?", [id], |r| {
                r.get(0)
            })
            .unwrap()
    }
}

fn text(out: &[u8]) -> String {
    String::from_utf8(out.to_vec()).unwrap()
}

#[test]
fn answer_list_and_dismiss_through_the_binary() {
    let rig = Rig::new();
    let out = rig.run(&["list"]);
    assert_eq!(out.status.code(), Some(0));
    let listed = text(&out.stdout);
    assert!(
        listed.contains("Q1") && listed.contains("Q2") && listed.contains("my-tab"),
        "{listed}"
    );

    let out = rig.run(&["answer", "Q1", "go", "ahead"]);
    assert_eq!(
        text(&out.stdout),
        "Q1 answered, delivered to w1:p1\n",
        "{}",
        text(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0));
    let prompts: Vec<Value> = rig.stub.params_of("agent.prompt");
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0]["text"].as_str().unwrap().ends_with(": go ahead"),
        "{prompts:?}"
    );
    let verified: Option<String> = open(&rig.db)
        .query_row(
            "SELECT verified_at FROM question_ext WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(verified.is_some(), "seen in the transcript");
    // the pane's token and its tab's count went down with it
    let tokens = rig.stub.params_of("pane.report_metadata");
    assert_eq!(tokens.last().unwrap()["tokens"], json!({"inbox_q": "?1"}));
    let renames = rig.stub.params_of("tab.rename");
    assert_eq!(renames.last().unwrap()["label"], "❓1 my-tab");

    let out = rig.run(&["dismiss", "2"]);
    assert_eq!(
        (out.status.code(), text(&out.stdout)),
        (Some(0), "Q2 dismissed\n".into())
    );
    assert_eq!(rig.status(2), "dismissed");
    let out = rig.run(&["dismiss", "2"]);
    assert_eq!(
        (out.status.code(), text(&out.stderr)),
        (Some(1), "Q2 is not open\n".into())
    );
    assert_eq!(
        rig.stub.params_of("pane.report_metadata").last().unwrap()["tokens"],
        json!({"inbox_q": null})
    );
}

#[test]
fn open_asks_herdr_for_the_popup_with_its_request_and_the_operators_pane() {
    let rig = Rig::new();
    let ctx = r#"{"focused_pane_id": "w1:p7"}"#;
    let out = rig.run_with(
        &["open", "--question", "Q2"],
        &[("HERDR_PLUGIN_CONTEXT_JSON", ctx)],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let opened = rig.stub.params_of("plugin.pane.open");
    assert_eq!(opened.len(), 1);
    let p = &opened[0];
    assert_eq!(
        (p["plugin_id"].as_str(), p["entrypoint"].as_str()),
        (Some("leavemyyard.herdr-inbox"), Some("inbox"))
    );
    // the binary runs from the test's target directory, no checkout: herdr's listed root
    assert_eq!(p["cwd"], "/inbox/root");
    assert_eq!(p["env"]["INBOX_FOCUS_PANE"], "w1:p7");
    let nonce = p["env"]["INBOX_OPEN_NONCE"].as_str().unwrap();
    let state = rig.db.parent().unwrap();
    let request = herdr_inbox::deeplink::request::consume_request(
        state,
        Some(nonce),
        herdr_inbox::deeplink::request::unix_now(),
    )
    .unwrap();
    assert_eq!(
        (request.kind.as_str(), request.value.as_str(), request.hold),
        ("question", "2", true)
    );
}

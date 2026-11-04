//! `ask_operator` and `check_answers` in a herdr pane, against the Python
//! server's recordings (`tests/fixtures/mcp/herdr-*.jsonl`, `tools/capture_mcp.py`).
//!
//! Python ran against the suite's fake herdr CLI; here the same herdr world is a
//! kit `FakeHerdr` socket. Each session must give the Python server's replies
//! exactly (receipts, `[inbox]` wording, the answers), leave the same question
//! rows, raise the same notifications and look up the same panes.
//!
//! Not compared: the pane markers (`inbox_q` token, the `❓N` tab prefix), which
//! are P5-G's, and herdr's `tab get`s, which Python's background marker job
//! interleaves with the tool's own.

#[allow(dead_code)] // the rig is shared with mcp_goldens.rs, which uses the rest of it
mod mcp_rig;

use std::path::Path;
use std::time::Duration;

use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_inbox::store::Store;
use mcp_rig::{golden, of, session};
use serde_json::{Value, json};

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn pane(pane: &str, tab: &str, session: Option<&str>) -> Reply {
    let mut p = json!({"pane_id": pane, "tab_id": tab, "workspace_id": "w1", "agent": "claude"});
    if let Some(s) = session {
        p["agent_session"] =
            json!({"agent": "claude", "kind": "id", "source": "herdr:claude", "value": s});
    }
    Reply::Result(json!({"pane": p, "type": "pane_info"}))
}

fn tab(tab: &str, label: &str) -> Reply {
    Reply::Result(
        json!({"tab": {"tab_id": tab, "label": label, "workspace_id": "w1"}, "type": "tab_info"}),
    )
}

/// The fake herdr every session starts from: pane w1:p1 in tab w1:t1 ("my-tab").
fn fake() -> FakeHerdr {
    let f = FakeHerdr::start();
    f.on("pane.get", pane("w1:p1", "w1:t1", Some(S)))
        .on("tab.get", tab("w1:t1", "my-tab"))
        .on(
            "agent.list",
            Reply::Result(json!({"agents": [], "type": "agent_list"})),
        )
        .on("notification.show", Reply::Result(json!({"type": "ok"})))
        .on("pane.report_metadata", Reply::Result(json!({"type": "ok"})));
    f
}

/// The operator answers between two requests.
fn act(action: &Value, dir: &Path) {
    let [qid, text] = action["answer"].as_array().unwrap().as_slice() else {
        panic!("{action}")
    };
    let store = Store::open(&dir.join("inbox.db"), Duration::from_secs(3)).unwrap();
    assert!(
        store
            .set_answer(qid.as_i64().unwrap(), text.as_str().unwrap(), None)
            .unwrap()
    );
}

/// The question rows as the capture writes them.
fn stored(dir: &Path) -> Value {
    let db = rusqlite::Connection::open(dir.join("inbox.db")).unwrap();
    let sql = "SELECT id, pane_id, tab_id, tab_label, session_id, cwd, asked_by, question, options, context, norm, \
               status, answer, repeat_count, acknowledged_at IS NOT NULL, delivery_error, header, multi_select, \
               group_id, options_json, herdr_server, title FROM questions JOIN question_ext USING (id) \
               LEFT JOIN question_groups USING (group_id) ORDER BY id";
    let mut stmt = db.prepare(sql).unwrap();
    // the server's cwd as it reads it: on macOS the temp dir is under a symlink (/var -> /private/var)
    let real = dir.canonicalize().unwrap();
    let cwds = [
        dir.to_string_lossy().into_owned(),
        real.to_string_lossy().into_owned(),
    ];
    let rows = stmt
        .query_map([], |r| {
            let mut row = Vec::new();
            for i in 0..22 {
                let v: rusqlite::types::Value = r.get(i)?;
                row.push(match v {
                    rusqlite::types::Value::Null => Value::Null,
                    rusqlite::types::Value::Integer(n) => json!(n),
                    rusqlite::types::Value::Text(t) if i == 5 && cwds.contains(&t) => {
                        json!("<cwd>")
                    }
                    rusqlite::types::Value::Text(_) if i == 20 => json!("<server>"),
                    rusqlite::types::Value::Text(t) => json!(t),
                    other => panic!("{other:?}"),
                });
            }
            Ok(Value::Array(row))
        })
        .unwrap();
    Value::Array(rows.map(Result::unwrap).collect())
}

/// Runs recording `name` against `herdr` (`None`: herdr unreachable).
fn check(name: &str, herdr: Option<&FakeHerdr>, extra: &[(&str, String)]) {
    let recs = golden(name);
    let dir = tempfile::tempdir().unwrap();
    Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(3)).unwrap();
    let socket = herdr.map_or_else(
        || dir.path().join("no-herdr.sock"),
        |f| f.socket().to_path_buf(),
    );
    let mut env = vec![
        ("HERDR_PANE_ID", "w1:p1".to_owned()),
        ("HERDR_SOCKET_PATH", socket.to_string_lossy().into_owned()),
    ];
    env.extend(extra.iter().cloned());
    let got = session(&recs, dir.path(), &env, &mut act);
    let want = of(&recs, "server->client");
    assert_eq!(got.len(), want.len(), "{name}: replies");
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g, w, "{name}: reply {}", w["id"]);
    }
    assert_eq!(
        stored(dir.path()),
        of(&recs, "store")[0],
        "{name}: questions as stored"
    );
    let python = of(&recs, "herdr");
    let argv = |prefix: [&str; 2]| -> Vec<Vec<Value>> {
        python
            .iter()
            .filter(|c| c[0] == prefix[0] && c[1] == prefix[1])
            .map(|c| c.as_array().unwrap()[2..].to_vec())
            .collect()
    };
    let Some(f) = herdr else { return };
    // the same notifications (Python: `notification show <title> --body <body> --sound request`)
    let shown: Vec<Vec<Value>> = f
        .params_of("notification.show")
        .into_iter()
        .map(|p| {
            vec![
                p["title"].clone(),
                json!("--body"),
                p["body"].clone(),
                json!("--sound"),
                p["sound"].clone(),
            ]
        })
        .collect();
    assert_eq!(
        shown,
        argv(["notification", "show"]),
        "{name}: notifications"
    );
    // the same panes looked up, in order; agent list as often
    let panes: Vec<Vec<Value>> = f
        .params_of("pane.get")
        .into_iter()
        .map(|p| vec![p["pane_id"].clone()])
        .collect();
    assert_eq!(panes, argv(["pane", "get"]), "{name}: pane lookups");
    assert_eq!(
        f.params_of("agent.list").len(),
        argv(["agent", "list"]).len(),
        "{name}: agent lists"
    );
}

#[test]
fn a_pane_asks_dedupes_gets_its_answer_once_and_files_a_group() {
    let f = fake();
    f.on("tab.get", tab("w1:t1", "\u{2753}2 my-tab")); // our mark is never part of the label
    check("herdr-pane", Some(&f), &[]);
}

#[test]
fn a_moved_pane_files_under_the_pane_that_runs_the_conversation() {
    let f = fake();
    f.once(
        "pane.get",
        Reply::error("pane_not_found", "pane w1:p1 not found"),
    )
    .on("pane.get", pane("w1:p2", "w1:t2", Some(S)))
    .on("tab.get", tab("w1:t2", "my-tab"))
    .on(
        "agent.list",
        Reply::Result(json!({"agents": [
                {"pane_id": "w1:p3", "agent_session": {"value": "other"}},
                {"pane_id": "w1:p2", "agent_session": {"value": S}}
            ], "type": "agent_list"})),
    );
    check(
        "herdr-moved",
        Some(&f),
        &[("CLAUDE_CODE_SESSION_ID", S.to_owned())],
    );
}

#[test]
fn without_herdrs_session_the_environments_is_used() {
    let f = fake();
    f.on("pane.get", pane("w1:p1", "w1:t1", None));
    check(
        "herdr-env-session",
        Some(&f),
        &[("CLAUDE_CODE_SESSION_ID", "claude-sess".to_owned())],
    );
}

#[test]
fn with_herdr_unreachable_the_question_is_still_filed_for_the_pane() {
    check(
        "herdr-down",
        None,
        &[("CLAUDE_CODE_SESSION_ID", S.to_owned())],
    );
}

//! The six tools in a herdr pane, against the Python
//! server's recordings (`tests/fixtures/mcp/herdr-*.jsonl`, `tools/capture_mcp.py`).
//!
//! Python ran against the suite's fake herdr CLI; here the same herdr world is a
//! kit `FakeHerdr` socket. Each session must give the Python server's replies
//! exactly (receipts, `[inbox]` wording, the answers), leave the same question
//! and alert rows, raise the same notifications and look up the same panes.
//!
//! Compared too: the markers' end state (each pane's last `inbox_q` count
//! token, each tab's last label). Not compared: herdr's `tab get`s, which
//! Python's background marker job interleaves with the tool's own.

#[allow(dead_code)] // the rig is shared with mcp_goldens.rs, which uses the rest of it
mod mcp_rig;

use std::path::Path;
use std::time::Duration;

use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_inbox::store::Store;
use mcp_rig::{cli_calls, golden, of, session};
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
        .on("pane.report_metadata", Reply::Result(json!({"type": "ok"})))
        .on("tab.rename", Reply::Result(json!({"type": "tab_info"})))
        // the suite's fake herdr CLI's panes (tests/conftest.py `PANE_TAB`)
        .on(
            "pane.list",
            Reply::Result(json!({"panes": [
                {"pane_id": "w1:p1", "tab_id": "w1:t1"},
                {"pane_id": "w1:p2", "tab_id": "w1:t2"},
                {"pane_id": "w1:p3", "tab_id": "w1:t1"}
            ], "type": "pane_list"})),
        );
    f
}

/// The operator answers between two requests.
fn act(action: &Value, dir: &Path) {
    let store = Store::open(&dir.join("inbox.db"), Duration::from_secs(3)).unwrap();
    if let Some(aid) = action.get("ack_alert") {
        assert!(
            store
                .acknowledge_alert(aid.as_i64().unwrap())
                .unwrap()
                .is_some()
        );
        return;
    }
    let [qid, text] = action["answer"].as_array().unwrap().as_slice() else {
        panic!("{action}")
    };
    assert!(
        store
            .set_answer(qid.as_i64().unwrap(), text.as_str().unwrap(), None)
            .unwrap()
    );
}

const QUESTIONS: &str = "SELECT id, pane_id, tab_id, tab_label, session_id, cwd, asked_by, question, options, context, norm, \
               status, answer, repeat_count, acknowledged_at IS NOT NULL, delivery_error, header, multi_select, \
               group_id, options_json, herdr_server, title FROM questions JOIN question_ext USING (id) \
               LEFT JOIN question_groups USING (group_id) ORDER BY id";
const ALERTS: &str = "SELECT id, kind, severity, title, message, pane_id, tab_id, tab_label, session_id, asked_by, \
                      herdr_server, status, acked_at IS NOT NULL, closed_at IS NOT NULL, reported_at IS NOT NULL \
                      FROM alerts ORDER BY id";

/// The rows `sql` reads, as the capture writes them: the cwd column `cwd_at`
/// as `<cwd>` when it is the server's, the server column `server_at` as `<server>`.
fn stored(dir: &Path, sql: &str, cwd_at: Option<usize>, server_at: usize) -> Value {
    let db = rusqlite::Connection::open(dir.join("inbox.db")).unwrap();
    let mut stmt = db.prepare(sql).unwrap();
    let width = stmt.column_count();
    // the server's cwd as it reads it: on macOS the temp dir is under a symlink (/var -> /private/var)
    let real = dir.canonicalize().unwrap();
    let cwds = [
        dir.to_string_lossy().into_owned(),
        real.to_string_lossy().into_owned(),
    ];
    let rows = stmt
        .query_map([], |r| {
            let mut row = Vec::new();
            for i in 0..width {
                let v: rusqlite::types::Value = r.get(i)?;
                row.push(match v {
                    rusqlite::types::Value::Null => Value::Null,
                    rusqlite::types::Value::Integer(n) => json!(n),
                    rusqlite::types::Value::Text(t) if Some(i) == cwd_at && cwds.contains(&t) => {
                        json!("<cwd>")
                    }
                    rusqlite::types::Value::Text(_) if i == server_at => json!("<server>"),
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
    check_labelled(name, herdr, extra, &[]);
}

/// [`check`] where the fake's tabs show `start_labels` (`(tab, label)`; others "my-tab").
fn check_labelled(
    name: &str,
    herdr: Option<&FakeHerdr>,
    extra: &[(&str, String)],
    start_labels: &[(&str, &str)],
) {
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
    let questions = stored(dir.path(), QUESTIONS, Some(5), 20);
    assert_eq!(questions, of(&recs, "store")[0], "{name}: questions");
    let alerts = stored(dir.path(), ALERTS, None, 10);
    assert_eq!(alerts, of(&recs, "alerts")[0], "{name}: alerts");
    let python = of(&recs, "herdr");
    let argv = |prefix: [&str; 2]| -> Vec<Vec<Value>> {
        python
            .iter()
            .filter(|c| c[0] == prefix[0] && c[1] == prefix[1])
            .map(|c| c.as_array().unwrap()[2..].to_vec())
            .collect()
    };
    // no real herdr CLI ran: the server's one CLI call (the plugin's settings, on the first
    // marker refresh) went to the rig's stub
    let cli = cli_calls(dir.path());
    assert!(!cli.is_empty(), "{name}: the stub was reached");
    assert!(
        cli.iter()
            .all(|c| c == "plugin config-dir leavemyyard.herdr-inbox"),
        "{name}: {cli:?}"
    );
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
    // the markers end as Python's: each pane's last count token, each tab's last label
    let want = &of(&recs, "markers")[0];
    let mut tokens = serde_json::Map::new();
    for p in f.params_of("pane.report_metadata") {
        let v = &p["tokens"]["inbox_q"];
        let token = v.as_str().map(|t| json!(format!("inbox_q={t}")));
        tokens.insert(
            p["pane_id"].as_str().unwrap().to_owned(),
            token.unwrap_or(Value::Null),
        );
    }
    assert_eq!(
        Value::Object(tokens),
        want["tokens"],
        "{name}: count tokens"
    );
    // the fake's label never changes, so a tab whose count it already shows is not renamed:
    // compare where each tab ends up (its last rename, else the label it had)
    let ends =
        |renames: Vec<(String, String)>, start: &dyn Fn(&str) -> String| -> Vec<(String, String)> {
            let mut out = std::collections::BTreeMap::new();
            for (tab, label) in renames {
                out.insert(tab, label);
            }
            let mut v: Vec<_> = out.into_iter().collect();
            v.retain(|(t, l)| *l != start(t));
            v
        };
    let ours: Vec<(String, String)> = f
        .params_of("tab.rename")
        .into_iter()
        .map(|p| {
            (
                p["tab_id"].as_str().unwrap().to_owned(),
                p["label"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let theirs: Vec<(String, String)> = want["labels"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(t, l)| (t.clone(), l.as_str().unwrap().to_owned()))
        .collect();
    let start = |tab: &str| -> String {
        start_labels
            .iter()
            .find(|(t, _)| *t == tab)
            .map_or("my-tab", |(_, l)| l)
            .to_owned()
    };
    assert_eq!(
        ends(ours, &start),
        ends(theirs, &start),
        "{name}: tab labels"
    );
}

#[test]
fn a_pane_asks_dedupes_gets_its_answer_once_and_files_a_group() {
    let f = fake();
    f.on("tab.get", tab("w1:t1", "\u{2753}2 my-tab")); // our mark is never part of the label
    check_labelled(
        "herdr-pane",
        Some(&f),
        &[],
        &[("w1:t1", "\u{2753}2 my-tab")],
    );
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

#[test]
fn the_callers_own_questions_and_alerts_are_listed_withdrawn_raised_and_reported() {
    check("herdr-tools", Some(&fake()), &[]);
}

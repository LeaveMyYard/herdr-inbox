//! hooks/hooks.json as Claude Code runs it: its wrapper around scripts/launch.

use std::process::{Command, Stdio};

use serde_json::Value;

use crate::world::{Root, repo, stand_in, stdout};

/// Every (event name in hooks.json, hook spec).
fn specs() -> Vec<(String, Value)> {
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(repo().join("hooks/hooks.json")).unwrap())
            .unwrap();
    let mut out = Vec::new();
    for (event, entries) in doc["hooks"].as_object().unwrap() {
        for entry in entries.as_array().unwrap() {
            for h in entry["hooks"].as_array().unwrap() {
                out.push((event.clone(), h.clone()));
            }
        }
    }
    out
}

fn run_spec(r: &Root, spec: &Value) -> std::process::Output {
    let root = r.root();
    let args: Vec<String> = spec["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            a.as_str()
                .unwrap()
                .replace("${CLAUDE_PLUGIN_ROOT}", root.to_str().unwrap())
        })
        .collect();
    Command::new(spec["command"].as_str().unwrap())
        .args(&args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("CLAUDE_PLUGIN_DATA", r.dir.path().join("data"))
        .env("XDG_STATE_HOME", r.dir.path().join("state"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn only_the_watchers_wake_code_becomes_claude_codes_exit_2() {
    for code in [0, 1, 2, 3, 4] {
        let r = Root::new();
        stand_in(
            &r.data_bin().join("herdr-inbox-1.0.0"),
            "bin",
            "1.0.0",
            1,
            code,
        );
        for (event, spec) in specs() {
            let wake = spec.get("asyncRewake").and_then(Value::as_bool) == Some(true);
            let out = run_spec(&r, &spec);
            let want = if wake && code == 3 { 2 } else { 0 };
            assert_eq!(out.status.code(), Some(want), "{event} exit {code}");
            assert!(out.stderr.is_empty(), "{event}: stderr is discarded");
        }
    }
}

#[test]
fn every_hook_reaches_the_binary_with_its_own_event() {
    // each Claude Code hook entry carries its own event name (PostToolBatch and Stop swapped
    // would deliver from the wrong hook)
    let r = Root::new();
    stand_in(
        &r.data_bin().join("herdr-inbox-1.0.0"),
        "bin",
        "1.0.0",
        1,
        0,
    );
    let mut seen: Vec<(String, String)> = specs()
        .iter()
        .map(|(event, s)| (event.clone(), stdout(&run_spec(&r, s)).trim().to_owned()))
        .collect();
    seen.sort();
    let want = [
        ("PostToolBatch", "bin post-tool-batch"),
        ("SessionStart", "bin session-start"),
        ("Stop", "bin rewake-watch"),
        ("Stop", "bin stop"),
        ("SubagentStart", "bin subagent-start"),
    ]
    .map(|(e, o)| (e.to_owned(), o.to_owned()));
    assert_eq!(seen, want);
}

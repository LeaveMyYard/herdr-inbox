//! The line a step writes. The variable itself is read by the binary, so
//! whether a step is written at all is tested through it
//! (tests/mcp_herdr.rs).

use super::*;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_line_holds_what_pythons_does_in_its_order() {
    let got = line("herdr notification show", 12.3456, &[("chars", json!(7))]);
    let v = parsed(&got);
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["t", "pid", "thread", "step", "ms", "chars"]);
    assert_eq!(v["step"], "herdr notification show");
    assert_eq!(v["ms"], 12.35);
    assert_eq!(v["chars"], 7);
    assert_eq!(v["pid"], std::process::id());
    assert!(v["t"].as_f64().unwrap() > 1_000_000_000.0);
    assert!(!v["thread"].as_str().unwrap().is_empty());
}

#[test]
fn a_named_thread_gives_its_name() {
    let got = std::thread::Builder::new()
        .name("inbox-send".into())
        .spawn(|| line("x", 0.0, &[]))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(parsed(&got)["thread"], "inbox-send");
}

#[test]
fn every_step_appends_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.jsonl");
    append(&path, "a", 1.0, &[]);
    append(&path, "b", 2.0, &[]);
    let text = std::fs::read_to_string(&path).unwrap();
    let steps: Vec<Value> = text.lines().map(parsed).collect();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1]["step"], "b");
}

#[test]
fn a_file_that_cannot_be_written_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    append(dir.path(), "a", 1.0, &[]); // a directory: never a panic
}

#[test]
fn a_call_is_named_as_python_names_its_cli_call() {
    for (op, step) in [
        ("notification.show", "herdr notification show"),
        ("pane.report_metadata", "herdr pane report-metadata"),
        ("server.reload_config", "herdr server reload-config"),
        ("agent.prompt", "herdr agent prompt"),
        ("plugin config-dir", "herdr plugin config-dir"),
        ("plugin.pane.open", "herdr plugin pane"),
    ] {
        assert_eq!(step_name(op), step);
    }
}

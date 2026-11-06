use std::path::Path;
use std::time::{Duration, Instant};

use super::*;
use crate::test_exe as exe;

fn stub(dir: &Path, body: &str) -> Cli {
    let path = dir.join("hc");
    exe::install(&path, format!("#!/bin/sh\n{body}\n"));
    Cli::new(path.into(), Duration::from_secs(2))
}

#[test]
fn get_and_ls_read_hcs_json_with_these_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let hc = stub(dir.path(), "printf '{\"args\":\"%s\"}' \"$*\"");
    assert_eq!(hc.get("t3").unwrap()["args"], "get --json t3");
    assert_eq!(hc.ls().unwrap()["args"], "ls --json");
}

#[test]
fn a_bad_address_never_reaches_hc() {
    let dir = tempfile::tempdir().unwrap();
    let hc = stub(dir.path(), "touch \"$0.ran\"; echo '{}'");
    assert_eq!(hc.get("--all"), Err("invalid core address"));
    assert!(!dir.path().join("hc.ran").exists());
}

#[test]
fn a_failure_is_an_answer_not_a_guess() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(stub(dir.path(), "exit 3").ls(), Err("hc read failed"));
    assert_eq!(
        stub(dir.path(), "echo not json").ls(),
        Err("hc output is not JSON")
    );
    let none = Cli::new(dir.path().join("no-hc").into(), Duration::from_secs(1));
    assert_eq!(none.ls(), Err("hc unavailable"));
}

#[test]
fn a_hanging_hc_is_given_up_on() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hc");
    exe::install(&path, "#!/bin/sh\nexec sleep 30\n");
    let hc = Cli::new(path.into(), Duration::from_millis(100));
    let start = Instant::now();
    assert_eq!(hc.ls(), Err("hc read timed out"));
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn too_much_output_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let hc = stub(dir.path(), "exec yes");
    assert_eq!(hc.ls(), Err("hc output exceeds limit"));
}

#[test]
fn the_child_never_sees_the_callers_terminal() {
    let hc = Cli::default();
    let command = hc.command(&["ls", "--json"]);
    let removed = command
        .get_envs()
        .any(|(key, value)| key == "HC_TERMINAL_ID" && value.is_none());
    assert!(removed);
}

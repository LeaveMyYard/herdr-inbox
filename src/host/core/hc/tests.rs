use std::fs::create_dir_all;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::{Duration, Instant};

use super::*;
use crate::test_exe as exe;

/// A core: something listening where `hc`'s socket would be. Never a real one.
fn core(dir: &Path) -> (PathBuf, UnixListener) {
    create_dir_all(dir).unwrap();
    let path = dir.join("c.sock");
    let listener = UnixListener::bind(&path).unwrap();
    (path, listener)
}

fn stub(dir: &Path, body: &str, socket: Option<PathBuf>) -> Cli {
    let path = dir.join("hc");
    exe::install(&path, format!("#!/bin/sh\n{body}\n"));
    Cli::new(path.into(), Duration::from_secs(2), socket)
}

#[test]
fn get_reads_hcs_json_with_these_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let hc = stub(
        dir.path(),
        "printf '{\"args\":\"%s\"}' \"$*\"",
        Some(socket),
    );
    assert_eq!(hc.get("t3").unwrap()["args"], "get --json t3");
}

#[test]
fn with_no_core_running_hc_is_never_run() {
    // any hc command would start a core, which keeps the caller's environment
    let dir = tempfile::tempdir().unwrap();
    let ran = "touch \"$0.ran\"; echo '{}'";
    let absent = dir.path().join("none.sock");
    assert_eq!(
        stub(dir.path(), ran, Some(absent)).get("t3"),
        Err("no core is running")
    );
    assert_eq!(
        stub(dir.path(), ran, None).get("t3"),
        Err("no known core socket")
    );
    assert!(!dir.path().join("hc.ran").exists());
}

#[test]
fn a_socket_nobody_listens_on_is_no_core() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, listener) = core(dir.path());
    drop(listener); // the file stays, as after a crash
    assert!(socket.exists());
    let hc = stub(dir.path(), "touch \"$0.ran\"", Some(socket));
    assert_eq!(hc.get("t3"), Err("no core is running"));
    assert!(!dir.path().join("hc.ran").exists());
}

#[test]
fn a_bad_address_never_reaches_hc() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let hc = stub(dir.path(), "touch \"$0.ran\"; echo '{}'", Some(socket));
    assert_eq!(hc.get("--all"), Err("invalid core address"));
    assert!(!dir.path().join("hc.ran").exists());
}

#[test]
fn a_failure_is_an_answer_not_a_guess() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let s = || Some(socket.clone());
    assert_eq!(
        stub(dir.path(), "exit 3", s()).get("t1"),
        Err("hc read failed")
    );
    assert_eq!(
        stub(dir.path(), "echo not json", s()).get("t1"),
        Err("hc output is not JSON")
    );
    let none = Cli::new(dir.path().join("no-hc").into(), Duration::from_secs(1), s());
    assert_eq!(none.get("t1"), Err("hc unavailable"));
}

#[test]
fn a_hanging_hc_is_given_up_on() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let path = dir.path().join("hc");
    exe::install(&path, "#!/bin/sh\nexec sleep 30\n");
    let hc = Cli::new(path.into(), Duration::from_millis(100), Some(socket));
    let start = Instant::now();
    assert_eq!(hc.get("t1"), Err("hc read timed out"));
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn too_much_output_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let hc = stub(dir.path(), "exec yes", Some(socket));
    assert_eq!(hc.get("t1"), Err("hc output exceeds limit"));
}

#[test]
fn the_child_never_sees_the_callers_terminal() {
    let hc = Cli::on_path(None);
    let command = hc.command(&["get", "--json", "t1"]);
    let removed = command
        .get_envs()
        .any(|(key, value)| key == "HC_TERMINAL_ID" && value.is_none());
    assert!(removed);
}

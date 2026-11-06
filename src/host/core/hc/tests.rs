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
    assert_eq!(stub(dir.path(), "exit 3", s()).get("t1"), Err(NO_TERMINAL));
    assert_eq!(
        stub(dir.path(), "exit 1", s()).get("t1"),
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
    let command = hc.command(Path::new("/x/c.sock"), &["get", "--json", "t1"]);
    let removed = command
        .get_envs()
        .any(|(key, value)| key == "HC_TERMINAL_ID" && value.is_none());
    assert!(removed);
}

/// A stub that prints its arguments and environment, and exits `code`.
fn recording(dir: &Path, code: i32, socket: PathBuf) -> Cli {
    let log = dir.join("log");
    let body = format!(
        "printf '%s\\n' \"$*\" \"sock=$HC_SOCKET\" \"t=$HC_TERMINAL_ID\" > {}\n\
         echo '{{\"error\":{{\"message\":\"no\"}}}}' >&2\nexit {code}",
        log.display()
    );
    stub(dir, &body, Some(socket))
}

#[test]
fn send_always_asks_for_idle_and_has_no_way_to_force() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let hc = recording(dir.path(), 0, socket.clone());
    assert_eq!(hc.send("t3", "[inbox] Q1: yes", "idle"), Sent::Typed);
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    let mut lines = log.lines();
    assert_eq!(
        lines.next(),
        Some("send --json --if idle t3 -- [inbox] Q1: yes")
    );
    assert_eq!(lines.next(), Some(&*format!("sock={}", socket.display())));
    assert!(!log.contains("--force"));
}

#[test]
fn a_text_that_looks_like_an_option_stays_text() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let hc = recording(dir.path(), 0, socket);
    hc.send("t3", "--force", "idle");
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert!(log.starts_with("send --json --if idle t3 -- --force\n"));
}

#[test]
fn each_refusal_of_hc_is_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let send = |code| recording(dir.path(), code, socket.clone()).send("t3", "x", "idle");
    assert_eq!(send(9), Sent::Blocked);
    assert_eq!(send(10), Sent::NotIdle);
    assert_eq!(send(3), Sent::Gone);
    assert_eq!(send(11), Sent::Gone);
    assert_eq!(send(1), Sent::Failed("hc send failed: no".into()));
}

#[test]
fn with_no_core_nothing_is_sent_and_a_bad_address_never_reaches_hc() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let ran = "touch \"$0.ran\"";
    assert_eq!(
        stub(dir.path(), ran, Some(dir.path().join("none.sock"))).send("t3", "x", "idle"),
        Sent::Unread("no core is running")
    );
    assert_eq!(
        stub(dir.path(), ran, None).send("t3", "x", "idle"),
        Sent::Unread("no known core socket")
    );
    assert_eq!(
        stub(dir.path(), ran, Some(socket)).send("--force", "x", "idle"),
        Sent::Unread("invalid core address")
    );
    assert!(!dir.path().join("hc.ran").exists());
}

#[test]
fn a_send_stopped_part_way_counts_as_typed_so_only_the_transcript_decides() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let path = dir.path().join("hc");
    exe::install(&path, "#!/bin/sh\nexec sleep 30\n");
    let hc = Cli::new(path.into(), Duration::from_secs(2), Some(socket))
        .with_send_timeout(Duration::from_millis(100));
    assert_eq!(hc.send("t3", "x", "idle"), Sent::Typed);
}

#[test]
fn a_missing_hc_typed_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let none = Cli::new(
        dir.path().join("no-hc").into(),
        Duration::from_secs(1),
        Some(socket),
    );
    assert_eq!(none.send("t1", "x", "idle"), Sent::Unread("hc unavailable"));
}

#[test]
fn read_takes_the_text_of_hcs_json() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let body = "printf '{\"complete\":true,\"how\":\"screen\",\"text\":\"%s\"}' \"$*\"";
    let hc = stub(dir.path(), body, Some(socket.clone()));
    assert_eq!(hc.screen("t3").unwrap(), "read --json t3");
    let hc = stub(dir.path(), "echo '{}'", Some(socket));
    assert_eq!(hc.screen("t3"), Err("hc read has no text"));
    assert_eq!(hc.screen("--all"), Err("invalid core address"));
}

#[test]
fn a_unit_test_has_no_real_hc_whatever_it_is_given() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let hc = Cli::on_path(Some(socket));
    assert_eq!(hc.get("t1"), Err("no known core socket"));
    assert_eq!(
        hc.send("t1", "x", "idle"),
        Sent::Unread("no known core socket")
    );
}

#[test]
fn hc_is_told_which_socket_was_tried() {
    let hc = Cli::new("hc".into(), Duration::from_secs(1), None);
    let command = hc.command(Path::new("/x/c.sock"), &["get"]);
    let told = command
        .get_envs()
        .any(|(key, value)| key == "HC_SOCKET" && value == Some("/x/c.sock".as_ref()));
    assert!(told);
}

#[test]
fn the_switch_changes_only_done_and_never_says_force() {
    for status in ["idle", "working", "error", "None"] {
        assert_eq!(
            send_args("t3", "x", status, true),
            ["send", "--json", "--if", "idle", "t3", "--", "x"]
        );
    }
    assert_eq!(
        send_args("t3", "x", "done", false),
        ["send", "--json", "--if", "idle", "t3", "--", "x"]
    );
    assert_eq!(
        send_args("t3", "x", "done", true),
        ["send", "--json", "t3", "--", "x"]
    );
}

#[test]
fn the_switch_stays_off_until_the_operator_says_otherwise() {
    let args = send_args("t3", "x", "done", PLAIN_SEND_AT_DONE);
    assert!(
        args.contains(&"--if"),
        "keep --if idle until the operator says"
    );
}

#[test]
fn list_is_this_machines_live_terminals() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, _core) = core(dir.path());
    let body = r#"echo '{"terminals":[{"address":"t1","state":"live"},{"address":"t2","state":"finished"},{"address":"vps/t7","state":"live"},{"address":"--all","state":"live"}]}'"#;
    assert_eq!(stub(dir.path(), body, Some(socket.clone())).list(), ["t1"]);
    assert!(stub(dir.path(), "exit 1", Some(socket)).list().is_empty());
}

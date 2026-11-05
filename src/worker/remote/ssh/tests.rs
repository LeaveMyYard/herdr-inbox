//! The ssh runner with stand-in programs (shell scripts this test writes):
//! exit codes, stdin, the timeout, a client that cannot start, and the
//! options each kind of call gets.

use std::fs;

use super::*;
use crate::remote::command::{FixedCommand, SafeTarget};
use crate::store::mirror::Machine;

fn script(dir: &std::path::Path, body: &str) -> OsString {
    let p = dir.join("fake-ssh");
    crate::worker::remote::tests::script(&p, body);
    p.into_os_string()
}

fn env(program: OsString, runtime: Option<PathBuf>, home: PathBuf) -> SshEnv {
    SshEnv {
        program,
        runtime_dir: runtime,
        home,
    }
}

/// A directory short enough for a control socket: macOS's default temporary
/// directory is not, and connection sharing is then rightly off.
fn short_dir() -> tempfile::TempDir {
    tempfile::tempdir_in("/tmp").unwrap()
}

#[test]
fn the_exit_code_stdout_stderr_and_stdin_come_back() {
    let dir = tempfile::tempdir().unwrap();
    let p = script(dir.path(), "cat; echo err >&2; exit 3");
    let r = exec(&p, &[], Some("{\"x\": 1}"), Duration::from_secs(5), false);
    assert_eq!(
        (r.code, r.stdout.as_str(), r.stderr.as_str()),
        (3, "{\"x\": 1}", "err\n")
    );
}

#[test]
fn a_call_past_its_time_is_killed_and_reads_124() {
    let dir = tempfile::tempdir().unwrap();
    let p = script(dir.path(), "echo partial; exec sleep 30");
    let started = Instant::now();
    let r = exec(&p, &[], None, Duration::from_millis(300), true);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        (r.code, r.stdout.as_str(), r.stderr.as_str()),
        (124, "partial\n", "timed out after 0 s")
    );
    let r = exec(&p, &[], Some(""), Duration::from_millis(300), false);
    assert_eq!((r.code, r.stdout.as_str()), (124, "")); // call(): no partial output
}

#[test]
fn a_client_that_cannot_start_reads_127() {
    let r = exec(
        &OsString::from("/nonexistent/ssh"),
        &[],
        None,
        Duration::from_secs(1),
        true,
    );
    assert_eq!(r.code, 127);
    assert!(!r.stderr.is_empty());
}

#[test]
fn connection_sharing_needs_a_short_existing_directory() {
    let dir = short_dir();
    let ok = env("ssh".into(), Some(dir.path().into()), "/nonexistent".into());
    let opts = control_options(&ok);
    assert_eq!(opts[1], "ControlMaster=auto");
    assert_eq!(
        opts[3],
        format!("ControlPath={}", dir.path().join("hinbox-%C").display())
    );
    assert_eq!(opts[5], "ControlPersist=60");
    let missing = env(
        "ssh".into(),
        Some("/nonexistent/run".into()),
        "/nonexistent".into(),
    );
    assert_eq!(control_options(&missing), ["-o", "ControlMaster=no"]);
    let deep = dir.path().join("d".repeat(60));
    fs::create_dir_all(&deep).unwrap();
    assert_eq!(
        control_options(&env("ssh".into(), Some(deep), "/x".into())),
        ["-o", "ControlMaster=no"]
    );
    // no runtime directory: ~/.ssh
    let home = short_dir();
    fs::create_dir(home.path().join(".ssh")).unwrap();
    let opts = control_options(&env("ssh".into(), Some(PathBuf::new()), home.path().into()));
    assert!(opts[3].starts_with(&format!(
        "ControlPath={}",
        home.path().join(".ssh").display()
    )));
}

#[test]
fn only_an_inbox_call_shares_the_connection_and_the_command_goes_last() {
    let dir = short_dir();
    let p = script(dir.path(), "printf '%s\\n' \"$@\"");
    let e = env(p, Some(dir.path().into()), "/x".into());
    let target = SafeTarget::for_ssh(Some("box.lan")).unwrap();
    let probe = run(&e, &target, &FixedCommand::probe(), None);
    let args: Vec<&str> = probe.stdout.lines().collect();
    assert_eq!(&args[..8], SSH_OPTIONS);
    assert_eq!(&args[8..10], ["--", "box.lan"]);
    assert_eq!(args[10..].join("\n"), FixedCommand::probe().as_str());
    let rec = Machine {
        uv: Some("/r/uv".into()),
        herdr: Some("/r/herdr".into()),
        plugin_root: Some("/r/root".into()),
        ..Machine::default()
    };
    let export = FixedCommand::inbox(&rec, crate::remote::command::InboxSub::Export).unwrap();
    let call = run(&e, &target, &export, Some("{}"));
    let args: Vec<&str> = call.stdout.lines().collect();
    assert!(
        args.contains(&"ControlMaster=auto") && args.ends_with(&["--", "box.lan", export.as_str()])
    );
}

#[test]
fn a_descendant_holding_the_pipes_never_holds_the_call() {
    let dir = tempfile::tempdir().unwrap();
    // exits at once, but leaves a child with its stdout and stderr
    let p = script(dir.path(), "sleep 3 & echo hi");
    let started = Instant::now();
    let r = exec(&p, &[], None, Duration::from_millis(500), true);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!((r.code, r.stdout.as_str()), (124, "hi\n"));
    let r = exec(&p, &[], Some(""), Duration::from_millis(500), false);
    assert_eq!((r.code, r.stdout.as_str()), (124, ""));
}

#[test]
fn a_call_still_running_at_its_time_is_killed_without_exec() {
    let dir = tempfile::tempdir().unwrap();
    let p = script(dir.path(), "echo partial; sleep 3");
    let started = Instant::now();
    let r = exec(&p, &[], None, Duration::from_millis(300), true);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!((r.code, r.stdout.as_str()), (124, "partial\n"));
}

#[test]
fn a_call_past_its_time_leaves_no_process_behind() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let p = script(
        dir.path(),
        &format!("echo $$ > '{}'; exec sleep 30", pid_file.display()),
    );
    let r = exec(&p, &[], None, Duration::from_millis(500), true);
    assert_eq!(r.code, 124);
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    // signal 0 only, on this test's own stand-in: a zombie would still answer it
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "the timed-out client is still there");
}

#[test]
fn only_the_probe_without_a_request_gets_the_probes_time() {
    let probe = FixedCommand::probe();
    assert_eq!(timeout_of(&probe, None), PROBE_TIMEOUT);
    assert_eq!(timeout_of(&probe, Some("")), CALL_TIMEOUT);
    let other = FixedCommand::plugin_list("/r/bin/herdr").unwrap();
    assert_eq!(timeout_of(&other, None), CALL_TIMEOUT);
}

//! Python tests/test_successor.py: the successor after an update, and the
//! newer worker a revision-wanted marker asks for.

use std::cell::Cell;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use herdr_agent_kit::agent_procs::pid_alive;

use super::super::*;
use super::fakes::{Fakes, wait_for, with_mode, write_exe};
use crate::worker::lock::hold;

fn uv_line(root: &std::path::Path) -> String {
    let here = std::env::current_dir().unwrap();
    format!(
        "uv|run --quiet --frozen --no-dev --project {} inbox worker|{}",
        root.display(),
        here.display()
    )
}

/// The pid a Rust stand-in logged.
fn rust_pid(f: &Fakes) -> u32 {
    let line = f
        .calls()
        .into_iter()
        .find(|l| l.starts_with("rust|"))
        .unwrap();
    line.rsplit('|').next().unwrap().parse().unwrap()
}

#[test]
fn no_binary_starts_the_python_worker_exactly_as_before() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    assert!(start_successor(&f.env(), Some(&root)));
    assert!(wait_for(|| !f.calls().is_empty()));
    assert_eq!(
        f.calls(),
        [uv_line(&root)],
        "no venv selection inherited, no cwd of its own"
    );
}

#[test]
fn no_root_or_no_uv_is_no_successor() {
    let f = Fakes::new();
    assert!(!start_successor(&f.env(), None));
    let mut env = f.env();
    let path = env.spawn_env.iter_mut().find(|(k, _)| k == "PATH").unwrap();
    path.1 = OsString::from(&f.bin);
    assert!(
        !start_successor(&env, Some(&f.root("checkout"))),
        "uv cannot start"
    );
}

#[test]
fn an_installed_rust_worker_takes_over_the_lock_and_no_python_starts() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    let binary = f.rust(&root, "target/release/herdr-inbox");
    let env = f.env();
    assert!(start_successor(&env, Some(&root)));
    assert!(is_running(&env.lock_path()));
    let line = format!(
        "rust|{}|worker|{}|{}",
        binary.display(),
        root.display(),
        rust_pid(&f)
    );
    assert_eq!(f.calls(), [line], "started from its root; no uv, no venv");
    assert!(
        hold(&env.lock_path()).unwrap().is_none(),
        "a Python worker cannot start now"
    );
}

#[test]
fn a_binary_without_a_worker_falls_back_to_python_at_once() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    f.rust(&root, "target/release/herdr-inbox");
    let mut env = with_mode(f.env(), "exit");
    env.waits.handover_wait = Duration::from_secs(30);
    let started = Instant::now();
    assert!(start_successor(&env, Some(&root)));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(wait_for(|| f.calls().len() == 2));
    assert!(f.calls()[0].starts_with("rust|"));
    assert_eq!(f.calls()[1], uv_line(&root));
}

#[test]
fn a_binary_that_never_takes_the_lock_is_killed_and_python_starts() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    f.rust(&root, "target/release/herdr-inbox");
    let mut env = with_mode(f.env(), "hang");
    env.waits.handover_wait = Duration::from_millis(500);
    assert!(start_successor(&env, Some(&root)));
    assert!(
        !pid_alive(Some(rust_pid(&f))),
        "killed and reaped, not left behind"
    );
    assert!(wait_for(|| f.calls().iter().any(|l| l.starts_with("uv|"))));
}

#[test]
fn a_rust_worker_that_finds_the_lock_taken_still_counts_as_handed_over() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    f.rust(&root, "target/release/herdr-inbox");
    let env = f.env();
    let _held = hold(&env.lock_path()).unwrap().unwrap();
    assert!(
        start_successor(&env, Some(&root)),
        "the lock is held: someone serves"
    );
    assert!(wait_for(|| f
        .calls()
        .iter()
        .any(|l| l.starts_with("rust|"))));
    assert!(
        wait_for(|| !pid_alive(Some(rust_pid(&f)))),
        "it exited: never both live"
    );
    assert!(f.calls().iter().all(|l| !l.starts_with("uv|")));
}

#[test]
fn exec_format_error_falls_back_to_python() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    write_exe(
        &root.join("target/release/herdr-inbox"),
        "\x7fELF\x02garbage",
    );
    assert!(start_successor(&f.env(), Some(&root)));
    assert!(wait_for(|| f.calls() == [uv_line(&root)]));
}

#[test]
fn a_worker_that_takes_the_lock_at_the_deadline_is_not_killed() {
    let child = spawn_detached("/bin/sleep".as_ref(), &["2".into()], None, &[], None).unwrap();
    let pid = child.id();
    let asked = Cell::new(0);
    let done = |_: &std::process::Child| {
        asked.set(asked.get() + 1);
        asked.get() > 1 // the lock is re-checked right before the kill
    };
    assert!(watch(child, Duration::ZERO, done, || false));
    assert!(pid_alive(Some(pid)), "still alive: it holds the lock");
}

#[test]
fn the_kill_takes_the_whole_process_group() {
    let f = Fakes::new();
    f.logger("uv");
    let root = f.root("checkout");
    let pidfile = f.dir.path().join("child.pid");
    let launcher = format!(
        "#!/bin/sh\nsleep 30 &\necho $! > {}\nwait\n",
        pidfile.display()
    );
    write_exe(&root.join("target/release/herdr-inbox"), &launcher);
    let mut env = f.env();
    env.waits.handover_wait = Duration::from_millis(500);
    assert!(start_successor(&env, Some(&root)));
    let child: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_for(|| !pid_alive(Some(child)) || zombie(child)),
        "the launcher's child survived"
    );
}

/// `pid` is a zombie (killed, not yet reaped by its new parent).
fn zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
        s.rsplit(')')
            .next()
            .is_some_and(|r| r.trim_start().starts_with('Z'))
    })
}

#[test]
fn a_marker_starts_the_listed_installs_rust_worker_if_it_stands_by() {
    let f = Fakes::new();
    f.logger("python3");
    let root = f.root("r2");
    f.rust(&root, "bin/herdr-inbox");
    spawn_newer(&with_mode(f.env(), "standby"), Some(&root));
    let calls = f.calls();
    assert!(
        calls.len() == 1 && calls[0].starts_with("rust|"),
        "{calls:?}: no Python started"
    );
}

#[test]
fn a_marker_falls_back_to_the_listed_installs_python_worker() {
    let f = Fakes::new();
    f.logger("python3");
    let root = f.root("r2");
    f.rust(&root, "bin/herdr-inbox"); // a release binary with no worker yet
    spawn_newer(&with_mode(f.env(), "exit"), Some(&root));
    assert!(wait_for(|| f.calls().len() == 2));
    let python = &f.calls()[1];
    assert!(python.starts_with("python3|-I -c import runpy"), "{python}");
    let tail = format!("{}/src worker|{}", root.display(), root.display());
    assert!(python.ends_with(&tail), "{python}: its src, from its root");
    spawn_newer(&f.env(), None);
    assert_eq!(f.calls().len(), 2, "no root: nothing started");
}

//! The worker's world as the environment gives it, and `ensure` starting a
//! stand-in executable (a script this test writes) as `<exe> worker`.

use std::fs;
use std::time::{Duration, Instant};

use super::*;

fn vars(dir: &Path, extra: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    let db = dir.join("state/inbox.db");
    let mut v: Vec<(OsString, OsString)> = vec![
        ("HERDR_INBOX_DB".into(), db.into()),
        ("HERDR_SOCKET_PATH".into(), dir.join("mine.sock").into()),
        ("VIRTUAL_ENV".into(), "/venv".into()),
        ("UV_PROJECT_ENVIRONMENT".into(), "/venv".into()),
    ];
    v.extend(
        extra
            .iter()
            .map(|(k, val)| (OsString::from(k), OsString::from(val))),
    );
    v
}

fn plugin_env(v: &[(OsString, OsString)]) -> PluginEnv {
    PluginEnv::from_vars(v.iter().cloned(), Some("/home/u".into()), 1000)
}

#[test]
fn the_worker_serves_this_processs_server_unless_told_another() {
    let dir = tempfile::tempdir().unwrap();
    let v = vars(dir.path(), &[]);
    let e = worker_env(
        &plugin_env(&v),
        v.clone(),
        None,
        Path::new("/nowhere/herdr-inbox"),
    );
    assert_eq!(e.server, dir.path().join("mine.sock"));
    assert_eq!(e.db, dir.path().join("state/inbox.db"));
    assert_eq!(e.revision, MIGRATION_REVISION);
    assert_eq!(e.version, env!("CARGO_PKG_VERSION")); // no checkout: the compiled version
    assert!(!e.no_worker);
    let keys: Vec<_> = e.spawn_env.iter().map(|(k, _)| k.clone()).collect();
    assert!(
        !keys
            .iter()
            .any(|k| k == "VIRTUAL_ENV" || k == "UV_PROJECT_ENVIRONMENT")
    );
    let other = worker_env(
        &plugin_env(&v),
        v.clone(),
        Some(OsStr::new("/run/other.sock")),
        Path::new("/x"),
    );
    assert_eq!(other.server, OsString::from("/run/other.sock"));
    let sock: Vec<_> = other
        .spawn_env
        .iter()
        .filter(|(k, _)| k == "HERDR_SOCKET_PATH")
        .collect();
    assert_eq!(
        sock,
        [&(
            OsString::from("HERDR_SOCKET_PATH"),
            OsString::from("/run/other.sock")
        )]
    );
    assert_ne!(e.lock_path(), other.lock_path()); // one worker per server
}

#[test]
fn no_worker_turns_starting_off() {
    let dir = tempfile::tempdir().unwrap();
    let v = vars(dir.path(), &[(NO_WORKER_VAR, "1")]);
    let e = worker_env(&plugin_env(&v), v.clone(), None, Path::new("/x"));
    assert!(e.no_worker);
    assert!(!ensure(&e, Path::new("/bin/false")));
    let empty = vars(dir.path(), &[(NO_WORKER_VAR, "")]);
    assert!(!worker_env(&plugin_env(&empty), empty.clone(), None, Path::new("/x")).no_worker);
}

#[test]
fn ensure_starts_exe_worker_for_the_server_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("started");
    let exe = dir.path().join("stand-in");
    crate::test_exe::install(
        &exe,
        format!(
            "#!/bin/sh\necho \"$* $HERDR_SOCKET_PATH\" >> '{}'\n",
            out.display()
        ),
    );
    let v = vars(dir.path(), &[]);
    let e = worker_env(
        &plugin_env(&v),
        v.clone(),
        Some(OsStr::new("/run/other.sock")),
        &exe,
    );
    assert!(ensure(&e, &exe));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !out.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fs::read_to_string(&out).unwrap().trim(),
        "worker /run/other.sock"
    );
}

#[test]
fn an_empty_server_name_is_this_processs_server() {
    let dir = tempfile::tempdir().unwrap();
    let v = vars(dir.path(), &[]);
    let e = worker_env(
        &plugin_env(&v),
        v.clone(),
        Some(OsStr::new("")),
        Path::new("/nowhere/herdr-inbox"),
    );
    assert_eq!(e.server, dir.path().join("mine.sock"));
}

/// A checkout at version `version`, with its binary at `target/release`.
fn checkout(root: &Path, version: &str) -> PathBuf {
    let bin = root.join("target/release");
    fs::create_dir_all(&bin).unwrap();
    fs::write(root.join("herdr-plugin.toml"), "").unwrap();
    fs::write(
        root.join("pyproject.toml"),
        format!("[project]\nname = \"x\"\nversion = \"{version}\"\n"),
    )
    .unwrap();
    bin.join("herdr-inbox")
}

#[test]
fn a_worker_in_a_checkout_runs_the_version_on_its_disk_not_the_compiled_one() {
    // pulled but not rebuilt: comparing the compiled version with the disk would hand over
    // to this same binary on every tick, forever
    let dir = tempfile::tempdir().unwrap();
    let exe = checkout(dir.path(), "99.1.2");
    let v = vars(dir.path(), &[]);
    let e = worker_env(&plugin_env(&v), v.clone(), None, &exe);
    assert_eq!(e.version, "99.1.2");
    assert_ne!(e.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn a_worker_in_a_dev_checkout_never_follows_the_root_herdr_lists() {
    // a dev worktree whose server lists another install: following it would hand this
    // worker over to other code, possibly older
    let dir = tempfile::tempdir().unwrap();
    let exe = checkout(&dir.path().join("dev"), "99.1.2");
    let listed = dir.path().join("installed");
    fs::create_dir_all(&listed).unwrap();
    let mut install = install_of(&exe);
    assert_eq!(install.own(), Some(dir.path().join("dev").as_path()));
    assert!(!install.follows(|| Some(listed.clone())));
    assert_eq!(
        install.current(|| Some(listed.clone())),
        Some(dir.path().join("dev"))
    );
}

#[test]
fn a_worker_in_the_listed_checkout_follows_it_and_one_with_no_checkout_always_does() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("installed");
    let exe = checkout(&root, "99.1.2");
    let moved = dir.path().join("update");
    let mut install = install_of(&exe);
    assert!(install.follows(|| Some(root.clone())));
    assert_eq!(install.current(|| Some(moved.clone())), Some(moved.clone()));
    let mut copy = install_of(&dir.path().join("copy/herdr-inbox"));
    assert_eq!(copy.own(), None);
    assert_eq!(copy.current(|| Some(moved.clone())), Some(moved));
}

use std::time::{Duration, Instant};

use super::*;
use crate::worker::spawn::deliver_args;

#[test]
fn the_detached_sender_runs_deliver_with_the_ids_and_force() {
    // test_spawn_delivery_runs_inbox_deliver_detached: a stand-in binary records its args
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let exe = dir.path().join("stand-in");
    crate::test_exe::install(
        &exe,
        format!(
            "#!/bin/sh\necho \"$@\" > {}.tmp && mv {0}.tmp {0}\n",
            out.display()
        ),
    );
    let h = LiveHandoff {
        herdr: Herdr::new(herdr_agent_kit::herdr_api::ServerId::new(
            dir.path().join("no-herdr.sock"),
        ))
        .with_bin("/nonexistent/herdr-cli"),
        state_dir: dir.path().to_path_buf(),
        server: String::new(),
        exe: Some(exe),
        env: PluginEnv::from_vars(Vec::<(String, String)>::new(), None, 1000),
    };
    h.spawn_delivery(&[7, 8], true);
    let t = Instant::now();
    while !out.exists() && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "deliver 7 8 --force\n"
    );
    assert_eq!(deliver_args(&[3], false), ["deliver", "3"]);
}

#[test]
fn a_sender_that_cannot_start_is_only_logged() {
    let dir = tempfile::tempdir().unwrap();
    let h = LiveHandoff {
        herdr: Herdr::new(herdr_agent_kit::herdr_api::ServerId::new(
            dir.path().join("no-herdr.sock"),
        ))
        .with_bin("/nonexistent/herdr-cli"),
        state_dir: dir.path().to_path_buf(),
        server: String::new(),
        exe: Some(dir.path().join("missing-binary")),
        env: PluginEnv::from_vars(Vec::<(String, String)>::new(), None, 1000),
    };
    h.spawn_delivery(&[1], false); // must not panic
}

/// A hand-off whose binary is a stand-in that writes its args and server to `out`.
fn recording(dir: &std::path::Path, out: &std::path::Path, extra: &[(&str, &str)]) -> LiveHandoff {
    let exe = dir.join("stand-in");
    crate::test_exe::install(
        &exe,
        format!(
            "#!/bin/sh\necho \"$@ $HERDR_SOCKET_PATH\" >> {}\n",
            out.display()
        ),
    );
    let mut vars: Vec<(String, String)> = vec![
        (
            "HERDR_INBOX_DB".into(),
            dir.join("inbox.db").display().to_string(),
        ),
        (
            "HERDR_SOCKET_PATH".into(),
            dir.join("mine.sock").display().to_string(),
        ),
    ];
    vars.extend(extra.iter().map(|(k, v)| ((*k).into(), (*v).into())));
    LiveHandoff {
        herdr: Herdr::new(herdr_agent_kit::herdr_api::ServerId::new(
            dir.join("no-herdr.sock"),
        ))
        .with_bin("/nonexistent/herdr-cli"),
        state_dir: dir.to_path_buf(),
        server: String::new(),
        exe: Some(exe),
        env: PluginEnv::from_vars(vars, None, 1000),
    }
}

fn wait_for(path: &std::path::Path) -> String {
    let t = Instant::now();
    while !path.exists() && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_send_starts_the_worker_of_this_server() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let h = recording(dir.path(), &out, &[]);
    h.ensure_worker();
    let mine = dir.path().join("mine.sock");
    assert_eq!(wait_for(&out), format!("worker {}\n", mine.display()));
}

#[test]
fn an_answer_for_another_server_starts_that_servers_worker() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let h = recording(dir.path(), &out, &[]);
    h.ensure(Some("/other/herdr.sock"));
    assert_eq!(wait_for(&out), "worker /other/herdr.sock\n");
}

#[test]
fn no_worker_keeps_the_popup_from_starting_one() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let h = recording(dir.path(), &out, &[("HERDR_INBOX_NO_WORKER", "1")]);
    h.ensure_worker();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!out.exists(), "a worker was started");
}

#[test]
fn opening_the_popup_starts_the_worker_off_its_thread() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let h = Arc::new(recording(dir.path(), &out, &[]));
    ensure_in_background(&h);
    assert!(wait_for(&out).starts_with("worker "));
}

#[test]
fn delivering_an_answer_of_another_server_starts_that_servers_worker() {
    use crate::store::questions::NewQuestion;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let mut h = recording(dir.path(), &out, &[]);
    h.server = "/mine.sock".into();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 123",
            ..NewQuestion::default()
        })
        .unwrap();
    store.set_server(q.id, Some("/other/herdr.sock")).unwrap();
    store.set_answer(q.id, "yes", None).unwrap();
    let q = store.get(q.id).unwrap().unwrap();
    h.deliver_here(&store, &[q], false).unwrap();
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if std::fs::read_to_string(&out).is_ok_and(|s| s.contains("worker /other/herdr.sock\n")) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no worker for the other server");
}

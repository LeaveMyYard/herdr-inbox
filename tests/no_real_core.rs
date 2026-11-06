//! No test may reach a real core, whatever the environment it runs in. A core
//! delivery types real keystrokes, into whatever `hc` finds.
//!
//! `.cargo/config.toml` gives every process `cargo test` starts a dead `HC_SOCKET`,
//! which goes before any default. These tests fail if that goes, and show that
//! a delivery in such a process runs no `hc` even with a default socket that
//! answers and an `hc` on its `PATH`.

use std::fs;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use herdr_agent_kit::plugin_env::PluginEnv;
use herdr_inbox::host::core::hc::socket;
use herdr_inbox::store::Store;
use herdr_inbox::store::hosts::{CORE_SERVER, CoreSeat};
use herdr_inbox::store::questions::NewQuestion;

#[path = "support/exe.rs"]
mod exe;

const SESSION: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn dead_socket() -> String {
    std::env::var("HC_SOCKET").expect("HC_SOCKET: .cargo/config.toml must set it for cargo test")
}

#[test]
fn every_test_process_has_a_dead_core_socket_that_is_no_default() {
    let dead = dead_socket();
    assert!(Path::new(&dead).is_absolute(), "{dead}");
    assert!(!dead.starts_with("/run/user/"), "{dead}");
    assert!(dead.ends_with("target/no-core.sock"), "{dead}");
    assert!(
        !Path::new(&dead).exists(),
        "{dead}: nothing may listen there"
    );
    let env = PluginEnv::from_env();
    assert_eq!(socket::from_env(&env).as_deref(), Some(Path::new(&dead)));
}

/// `deliver` of a core row, in a process that inherits the test's environment,
/// with a listening default socket and an `hc` that says if it ran.
#[test]
fn a_delivery_started_by_a_test_never_runs_hc_even_with_a_default_core_up() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("run");
    fs::create_dir_all(runtime.join("herdr-core")).unwrap();
    let _default = UnixListener::bind(runtime.join("herdr-core/default.sock")).unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let ran = dir.path().join("hc.ran");
    exe::install(
        &bin.join("hc"),
        format!("#!/bin/sh\ntouch {}\necho '{{}}'\n", ran.display()),
    );
    let db = dir.path().join("inbox.db");
    let store = Store::open_migrating(&db, Duration::from_secs(5)).unwrap();
    let q = store
        .insert(&NewQuestion {
            session_id: Some(SESSION),
            question: "Merge it?",
            ..NewQuestion::default()
        })
        .unwrap();
    store
        .db()
        .execute(
            "UPDATE question_ext SET herdr_server = ? WHERE id = ?",
            (CORE_SERVER, q.id),
        )
        .unwrap();
    store
        .set_question_host(
            q.id,
            &CoreSeat {
                terminal: "t3".into(),
                machine: None,
            },
        )
        .unwrap();
    store.set_answer(q.id, "yes", None).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_herdr-inbox"))
        .args(["deliver", &q.id.to_string()])
        .env("HERDR_INBOX_DB", &db)
        .env("HERDR_INBOX_NO_WORKER", "1")
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("HC_SESSION")
        .env_remove("HC_TERMINAL_ID")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        !ran.exists(),
        "hc ran: a test could have reached a real core"
    );
    let error = store.get(q.id).unwrap().unwrap().delivery_error.unwrap();
    assert!(error.contains("no core is running"), "{error}");
}

//! The remote thread's round against stand-ins: a herdr whose `machine
//! list` is a script, an ssh script that plays the remote (the paths probe,
//! `plugin list`, `inbox export`), and a temporary database. herdr's socket
//! is a path nobody listens on: the notifications go nowhere.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use herdr_agent_kit::herdr_api::ServerId;
use serde_json::json;

use super::*;

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn herdr(dir: &Path, machines: &str) -> Herdr {
    let bin = dir.join("herdr");
    let log = dir.join("herdr.log");
    script(
        &bin,
        &format!(
            "echo \"$@\" >> '{}'\ncat <<'EOF'\n{machines}\nEOF",
            log.display()
        ),
    );
    Herdr::new(ServerId::new(dir.join("nobody.sock"))).with_bin(bin)
}

#[test]
fn sync_seconds_is_read_as_python_reads_it() {
    let s =
        |v: serde_json::Value| sync_seconds(json!({"remote_sync_seconds": v}).as_object().unwrap());
    assert_eq!(s(json!(2)), 2.0);
    assert_eq!(s(json!(0.5)), 0.5);
    assert_eq!(s(json!("3")), 3.0);
    assert_eq!(s(json!(0)), SYNC_SECONDS); // `or 5`
    assert_eq!(s(json!(true)), 1.0);
    assert_eq!(s(json!(-1)), SYNC_SECONDS);
    assert_eq!(s(json!("soon")), SYNC_SECONDS);
    assert_eq!(sync_seconds(&Map::new()), SYNC_SECONDS);
}

#[test]
fn only_enabled_machines_count_and_the_list_is_reused() {
    let dir = tempfile::tempdir().unwrap();
    let list = r#"[{"id": "m1", "label": "box", "target": "box.lan", "enabled": true},
                  {"id": "m2", "label": "off", "target": "off.lan", "enabled": false}, "junk"]"#;
    let h = herdr(dir.path(), list);
    let mut machines = Machines::default();
    let found = machines.enabled(&h);
    assert_eq!(
        found.iter().map(|m| m.id.as_deref()).collect::<Vec<_>>(),
        [Some("m1")]
    );
    machines.enabled(&h);
    let calls = fs::read_to_string(dir.path().join("herdr.log")).unwrap();
    assert_eq!(calls.lines().count(), 1, "{calls}"); // within MACHINES_TTL: asked once
    assert!(calls.contains("machine list --json"));
}

#[test]
fn a_herdr_that_cannot_list_means_no_machines() {
    let dir = tempfile::tempdir().unwrap();
    let h = Herdr::new(ServerId::new(dir.path().join("nobody.sock")))
        .with_bin(dir.path().join("missing"));
    assert!(Machines::default().enabled(&h).is_empty());
}

const EXPORT: &str = r#"{"version": "0.10.6", "format": 1, "db_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "seq": 5, "full": true,
 "questions": [{"id": 1, "qkey": "00000001", "group": 1, "status": "open", "question": "Ship it?", "options": ["yes", "no"]}],
 "ids": [1], "alerts": []}"#;

fn remote_env(dir: &Path) -> RemoteEnv {
    let ssh = dir.join("ssh");
    script(
        &ssh,
        &format!(
            "for a; do cmd=\"$a\"; done\necho \"$cmd\" >> '{log}'\n\
             case \"$cmd\" in\n\
             *'command -v'*) printf '/r/bin/uv\\n/r/bin/herdr\\n/r/bin/claude\\n' ;;\n\
             *'plugin list --json') echo '{{\"result\": {{\"plugins\": [{{\"plugin_id\": \"leavemyyard.herdr-inbox\", \"plugin_root\": \"/r/root\"}}]}}}}' ;;\n\
             *' inbox export') cat > /dev/null; cat <<'EOF'\n{EXPORT}\nEOF\n ;;\n\
             *) exit 9 ;;\nesac",
            log = dir.join("ssh.log").display()
        ),
    );
    let db = dir.join("inbox.db");
    drop(Store::open_migrating(&db, Duration::from_secs(5)).unwrap());
    RemoteEnv {
        db,
        herdr: Herdr::new(ServerId::new(dir.join("nobody.sock"))).with_bin(dir.join("no-herdr")),
        ssh: SshEnv {
            program: ssh.into_os_string(),
            runtime_dir: None,
            home: dir.to_path_buf(),
        },
        config_dir: None,
    }
}

fn machine(id: &str, target: &str) -> Machine {
    serde_json::from_value(json!({"id": id, "label": "box", "target": target, "enabled": true}))
        .unwrap()
}

#[test]
fn a_round_pulls_every_machine_into_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let env = remote_env(dir.path());
    sync_all(
        &env,
        &[machine("m1", "box.lan"), machine("m2", "-oProxyCommand=x")],
    );
    let store = Store::open(&env.db, Duration::from_secs(5)).unwrap();
    let ok: Option<String> = store
        .db()
        .query_row(
            "SELECT last_ok_at FROM remote_machines WHERE machine_id = 'm1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ok.is_some());
    let n: i64 = store
        .db()
        .query_row(
            "SELECT COUNT(*) FROM remote_questions WHERE machine_id = 'm1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    // an unsafe target never reaches ssh; its pull fails on its own
    let err: Option<String> = store
        .db()
        .query_row(
            "SELECT last_error FROM remote_machines WHERE machine_id = 'm2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(err.is_some());
    let calls = fs::read_to_string(dir.path().join("ssh.log")).unwrap();
    assert!(
        !calls.contains("ProxyCommand") && calls.lines().count() == 3,
        "{calls}"
    );
}

#[test]
fn a_machine_without_an_id_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let env = remote_env(dir.path());
    let m: Machine =
        serde_json::from_value(json!({"label": "x", "target": "box.lan", "enabled": true}))
            .unwrap();
    sync_all(&env, &[m]);
    assert!(!dir.path().join("ssh.log").exists());
}

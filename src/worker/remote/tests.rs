//! The remote thread's round against stand-ins: a herdr whose `machine
//! list` is a script, an ssh script that plays the remote (the paths probe,
//! `plugin list`, `inbox export`), and a temporary database. herdr's socket
//! is a path nobody listens on, unless a test records the notifications.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use herdr_agent_kit::herdr_api::ServerId;
use serde_json::json;

use super::*;

/// Writes an executable stand-in script and runs it once (it exits at once
/// under `STAND_IN_SETTLE`) until it starts: another test forking while this
/// one wrote the file can hold its write descriptor until that child execs,
/// and running the file meanwhile fails with ETXTBSY. Once it ran, no such
/// descriptor is left.
pub(in crate::worker::remote) fn script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ -n \"$STAND_IN_SETTLE\" ] && exit 0\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..500 {
        match std::process::Command::new(path)
            .env("STAND_IN_SETTLE", "1")
            .status()
        {
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(5)),
            _ => return,
        }
    }
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
    assert_eq!(s(json!(1e300)), SYNC_SECONDS); // too long to sleep for
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
            "for a; do target=\"$cmd\"; cmd=\"$a\"; done\n\
             [ \"$target\" = slow.lan ] && sleep 2\n\
             echo \"$target $cmd\" >> '{log}'\n\
             [ -e '{down}' ] && exit 255\n\
             case \"$cmd\" in\n\
             *'command -v'*) printf '/r/bin/uv\\n/r/bin/herdr\\n/r/bin/claude\\n' ;;\n\
             *'plugin list --json') echo '{{\"result\": {{\"plugins\": [{{\"plugin_id\": \"leavemyyard.herdr-inbox\", \"plugin_root\": \"/r/root\"}}]}}}}' ;;\n\
             *' inbox export') cat > /dev/null; cat <<'EOF'\n{EXPORT}\nEOF\n ;;\n\
             *) exit 9 ;;\nesac",
            log = dir.join("ssh.log").display(),
            down = dir.join("down").display()
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
    sync_all(&env, || {
        vec![machine("m1", "box.lan"), machine("m2", "-oProxyCommand=x")]
    });
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
    sync_all(&env, || vec![m]);
    assert!(!dir.path().join("ssh.log").exists());
}

#[test]
fn a_slow_machine_never_holds_up_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let env = remote_env(dir.path());
    sync_all(&env, || {
        vec![machine("m1", "slow.lan"), machine("m2", "box.lan")]
    });
    let calls = fs::read_to_string(dir.path().join("ssh.log")).unwrap();
    let lines: Vec<&str> = calls.lines().collect();
    let first = |target: &str| lines.iter().position(|l| l.starts_with(target)).unwrap();
    let box_done = lines
        .iter()
        .rposition(|l| l.starts_with("box.lan"))
        .unwrap();
    // the slow one's first call is logged after it waited: the other was done by then
    assert!(box_done < first("slow.lan"), "{calls}");
}

#[test]
fn a_failed_pull_flushes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let env = remote_env(dir.path());
    sync_all(&env, || vec![machine("m1", "box.lan")]);
    let store = Store::open(&env.db, Duration::from_secs(5)).unwrap();
    let data = json!({"id": 1, "qkey": "00000001", "_db_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"});
    let queued = store
        .enqueue(
            "m1",
            data.as_object().unwrap(),
            "answer",
            Some(&json!({"choices": ["yes"]})),
            "r1",
            &mut SystemTime::now,
            Duration::from_secs(3600),
        )
        .unwrap();
    assert!(queued.is_ok(), "{queued:?}");
    fs::write(dir.path().join("down"), "").unwrap(); // the machine is unreachable now
    sync_all(&env, || vec![machine("m1", "box.lan")]);
    let calls = fs::read_to_string(dir.path().join("ssh.log")).unwrap();
    assert!(
        calls.lines().last().unwrap().ends_with(" inbox export"),
        "{calls}"
    );
    assert!(!calls.contains(" inbox apply"), "{calls}");
}

/// A herdr socket that records every `notification.show`'s params.
fn recording_herdr(dir: &Path) -> (Herdr, Arc<Mutex<Vec<Value>>>) {
    use std::io::{BufRead, BufReader, Write};
    let socket = dir.join("notes.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&notes);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            let Ok(request) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if request["method"] == "notification.show" {
                seen.lock().unwrap().push(request["params"].clone());
            }
            let _ = writeln!(stream, "{}", json!({"id": request["id"], "result": {}}));
        }
    });
    (
        Herdr::new(ServerId::new(socket)).with_bin(dir.join("no-herdr")),
        notes,
    )
}

/// A queued action on question 1, past its time and never sent.
fn expired_row(env: &RemoteEnv, id: &str) {
    expired_row_of(env, id, 1);
}

/// A queued action on question `remote_id`: a flush takes one head per question.
fn expired_row_of(env: &RemoteEnv, id: &str, remote_id: i64) {
    let store = Store::open(&env.db, Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute(
            "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at, \
             expires_at, state) VALUES (?, 'm1', 'dd', ?, 'k', 'answer', 'c', '2020-01-01T00:00:00+00:00', 'queued')",
            rusqlite::params![id, remote_id],
        )
        .unwrap();
}

fn state_of(env: &RemoteEnv, id: &str) -> String {
    let store = Store::open(&env.db, Duration::from_secs(5)).unwrap();
    store
        .db()
        .query_row(
            "SELECT state FROM remote_outbox WHERE request_id = ?",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_new_remote_question_alerts_the_operator_with_the_request_sound() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = remote_env(dir.path());
    let (herdr, notes) = recording_herdr(dir.path());
    env.herdr = herdr;
    sync_all(&env, || vec![machine("m1", "box.lan")]);
    let notes = notes.lock().unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["sound"], "request");
}

#[test]
fn the_sweep_runs_before_the_machines_are_asked_for_even_with_none() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = remote_env(dir.path());
    let (herdr, notes) = recording_herdr(dir.path());
    env.herdr = herdr;
    expired_row(&env, "late");
    let swept = Mutex::new(None);
    sync_all(&env, || {
        *swept.lock().unwrap() = Some(state_of(&env, "late"));
        Vec::new()
    });
    let state = swept.lock().unwrap().clone().unwrap();
    assert_ne!(state, "queued");
    assert_eq!(notes.lock().unwrap().len(), 1);
}

#[test]
fn what_a_flush_reports_reaches_the_operator() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = remote_env(dir.path());
    let (herdr, notes) = recording_herdr(dir.path());
    env.herdr = herdr;
    sync_all(&env, || {
        expired_row(&env, "late"); // after the sweep: only the flush can report it
        vec![machine("m1", "box.lan")]
    });
    assert_ne!(state_of(&env, "late"), "queued");
    assert_eq!(notes.lock().unwrap().len(), 2, "the pull's and the flush's");
}

/// Fails `event` on `table` for the rows `when` selects (#132).
fn fault(env: &RemoteEnv, event: &str, table: &str, when: &str) {
    Store::open(&env.db, Duration::from_secs(5))
        .unwrap()
        .db()
        .execute_batch(&format!(
            "CREATE TRIGGER fault BEFORE {event} ON {table} WHEN {when} \
             BEGIN SELECT RAISE(ABORT, 'injected fault'); END"
        ))
        .unwrap();
}

#[test]
fn a_sweep_stopped_by_an_error_still_reports_the_row_it_settled() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = remote_env(dir.path());
    let (herdr, notes) = recording_herdr(dir.path());
    env.herdr = herdr;
    expired_row(&env, "first");
    expired_row(&env, "second");
    fault(&env, "UPDATE", "remote_outbox", "NEW.request_id = 'second'");
    sync_all(&env, Vec::new);
    assert_ne!(state_of(&env, "first"), "queued");
    assert_eq!(state_of(&env, "second"), "queued");
    assert_eq!(notes.lock().unwrap().len(), 1);
}

#[test]
fn a_flush_stopped_by_an_error_still_reports_the_row_it_settled() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = remote_env(dir.path());
    let (herdr, notes) = recording_herdr(dir.path());
    env.herdr = herdr;
    sync_all(&env, || {
        // after the sweep: only the flush can report them, each the head of its question
        expired_row_of(&env, "first", 1);
        expired_row_of(&env, "second", 2);
        expired_row_of(&env, "third", 3);
        fault(&env, "UPDATE", "remote_outbox", "NEW.request_id = 'second'");
        vec![machine("m1", "box.lan")]
    });
    assert_ne!(state_of(&env, "first"), "queued");
    assert_eq!(state_of(&env, "second"), "queued");
    assert_eq!(
        state_of(&env, "third"),
        "queued",
        "the fault stopped the flush"
    );
    assert_eq!(notes.lock().unwrap().len(), 2, "the pull's and the flush's");
}

#[test]
fn a_pull_stopped_by_an_error_reports_what_it_mirrored_and_flushes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = remote_env(dir.path());
    let (herdr, notes) = recording_herdr(dir.path());
    env.herdr = herdr;
    sync_all(&env, || {
        expired_row(&env, "waiting"); // after the sweep: a flush would settle it
        let store = Store::open(&env.db, Duration::from_secs(5)).unwrap();
        // the stuck-answer scan after the mirror write fails on this corrupt row
        store
            .db()
            .execute(
                "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
                 VALUES ('m9', 'dd', 1, 'k', 'answered', '{', 't')",
                [],
            )
            .unwrap();
        vec![machine("m1", "box.lan")]
    });
    assert_eq!(notes.lock().unwrap().len(), 1, "the new question's");
    assert_eq!(
        state_of(&env, "waiting"),
        "queued",
        "no flush after an error"
    );
}

#[test]
fn a_notification_is_capped_at_200_and_300_characters() {
    let dir = tempfile::tempdir().unwrap();
    let (herdr, notes) = recording_herdr(dir.path());
    let long = Notice {
        title: "Question Title 123 ".repeat(40),
        body: "Question Text 345 ".repeat(40),
    };
    let short = Notice {
        title: "Question Title 1".into(),
        body: "Question Text 1".into(),
    };
    notify(&herdr, vec![long, short]);
    let notes = notes.lock().unwrap();
    let chars = |n: usize, key: &str| notes[n][key].as_str().unwrap().chars().count();
    assert_eq!(
        (chars(0, "title"), chars(0, "body")),
        (200, 300),
        "{notes:?}"
    );
    assert_eq!(notes[1]["title"], "Question Title 1");
    assert_eq!(notes[1]["body"], "Question Text 1");
}

#[test]
fn a_panic_on_one_machine_leaves_the_others_their_turn() {
    let done = Mutex::new(Vec::new());
    let found = [machine("m1", "a.lan"), machine("m2", "b.lan")];
    each_machine(&found, &|m| {
        assert_ne!(m.id.as_deref(), Some("m1"), "this machine panics");
        done.lock().unwrap().push(m.id.clone());
    });
    assert_eq!(*done.lock().unwrap(), [Some("m2".to_string())]);
}

#[test]
fn a_panic_in_the_round_never_reaches_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let env = remote_env(dir.path());
    guarded_round(&env, || panic!("Question Text 123"));
}

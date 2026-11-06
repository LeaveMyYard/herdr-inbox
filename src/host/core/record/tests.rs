use std::fs;
use std::os::unix::fs::symlink;

use serde_json::json;

use super::*;

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

fn record() -> serde_json::Value {
    json!({"pid": 20, "sessionId": S, "procStart": "s20", "kind": "interactive", "cwd": "/r"})
}

fn written(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("sessions")).unwrap();
    fs::write(dir.path().join("sessions/20.json"), body).unwrap();
    dir
}

#[test]
fn the_record_of_this_exact_process_names_its_session() {
    let dir = written(&record().to_string());
    assert_eq!(session(dir.path(), 20, "s20"), Record::Names(S.into()));
}

#[test]
fn a_record_of_a_reused_pid_or_another_kind_is_unusable() {
    let dir = written(&record().to_string());
    assert_eq!(session(dir.path(), 20, "other"), Record::Unusable);
    for (key, value) in [
        ("pid", json!(21)),
        ("procStart", json!(1)),
        ("kind", json!("bg")),
        ("sessionId", json!("not-a-session")),
    ] {
        let mut r = record();
        r[key] = value;
        let dir = written(&r.to_string());
        assert_eq!(session(dir.path(), 20, "s20"), Record::Unusable, "{key}");
    }
}

#[test]
fn only_a_file_that_is_not_there_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(session(dir.path(), 20, "s20"), Record::Missing);
    fs::create_dir(dir.path().join("sessions")).unwrap();
    assert_eq!(session(dir.path(), 20, "s20"), Record::Missing);
}

#[test]
fn an_empty_file_junk_a_large_file_or_a_link_is_unusable() {
    for body in [
        String::new(),
        "[1]".into(),
        " ".repeat(MAX_BYTES as usize + 1),
    ] {
        let dir = written(&body);
        assert_eq!(session(dir.path(), 20, "s20"), Record::Unusable);
    }
    let dir = written(&record().to_string());
    let real = dir.path().join("sessions/20.json");
    fs::rename(&real, dir.path().join("elsewhere.json")).unwrap();
    symlink(dir.path().join("elsewhere.json"), &real).unwrap();
    assert_eq!(session(dir.path(), 20, "s20"), Record::Unusable);
}

#[test]
fn others_lists_every_record_but_this_process() {
    let dir = written(&record().to_string());
    let sessions = dir.path().join("sessions");
    let mut other = record();
    other["pid"] = json!(21);
    fs::write(sessions.join("21.json"), other.to_string()).unwrap();
    let mut daemon = record();
    daemon["pid"] = json!(22);
    daemon["kind"] = json!("bg");
    fs::write(sessions.join("22.json"), daemon.to_string()).unwrap();
    fs::write(sessions.join("23.json"), "").unwrap();
    fs::write(sessions.join("24.json"), record().to_string()).unwrap(); // says pid 20
    fs::write(sessions.join("notes.txt"), "x").unwrap();
    let mut got = others(dir.path(), 20);
    got.sort_by_key(|o| format!("{o:?}"));
    assert_eq!(
        got,
        [
            Other::Elsewhere,
            Other::Runs(21, "s20".into(), S.into()),
            Other::Unreadable(23),
            Other::Unreadable(24),
        ]
    );
}

#[test]
fn no_sessions_directory_is_no_others() {
    let dir = tempfile::tempdir().unwrap();
    assert!(others(dir.path(), 20).is_empty());
}

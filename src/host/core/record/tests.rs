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
    assert_eq!(session(dir.path(), 20, "s20").as_deref(), Some(S));
}

#[test]
fn a_record_of_a_reused_pid_or_another_kind_is_no_record() {
    let dir = written(&record().to_string());
    assert_eq!(
        session(dir.path(), 20, "other"),
        None,
        "another start token"
    );
    for (key, value) in [
        ("pid", json!(21)),
        ("procStart", json!(1)),
        ("kind", json!("bg")),
        ("sessionId", json!("not-a-session")),
    ] {
        let mut r = record();
        r[key] = value;
        let dir = written(&r.to_string());
        assert_eq!(session(dir.path(), 20, "s20"), None, "{key}");
    }
}

#[test]
fn no_file_a_link_junk_or_a_large_file_is_no_record() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(session(dir.path(), 20, "s20"), None);
    let dir = written("[1]");
    assert_eq!(session(dir.path(), 20, "s20"), None);
    let dir = written(&" ".repeat(MAX_BYTES as usize + 1));
    assert_eq!(session(dir.path(), 20, "s20"), None);
    let dir = written(&record().to_string());
    let real = dir.path().join("sessions/20.json");
    fs::rename(&real, dir.path().join("elsewhere.json")).unwrap();
    symlink(dir.path().join("elsewhere.json"), &real).unwrap();
    assert_eq!(session(dir.path(), 20, "s20"), None);
}

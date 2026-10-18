use std::fs;
use std::path::PathBuf;

use super::*;

const OLD: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const NEW: &str = "aaaaaaaa-0000-4000-8000-000000000002";

fn continued(to: &str, ts: &str) -> String {
    format!(
        r#"{{"type":"continued-in","sessionId":"{OLD}","continuedInSessionId":"{to}","timestamp":"{ts}"}}"#
    )
}

fn record(kind: &str, ts: &str) -> String {
    format!(
        r#"{{"type":"{kind}","sessionId":"{OLD}","timestamp":"{ts}","message":{{"content":"hi"}}}}"#
    )
}

fn transcript(dir: &std::path::Path, name: &str, lines: &[String]) -> PathBuf {
    let path = dir.join(format!("{name}.jsonl"));
    fs::write(
        &path,
        lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
    .unwrap();
    path
}

#[test]
fn a_continued_in_record_continues_the_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let lines = [
        record("user", "2026-10-16T11:00:00Z"),
        continued(NEW, "2026-10-16T11:22:00Z"),
        record("last-prompt", "2026-10-16T11:23:00Z"),
    ];
    let got = continued_in(&transcript(dir.path(), OLD, &lines));
    assert_eq!(got.as_ref().map(|s| s.as_str()), Some(NEW));
}

#[test]
fn a_later_conversation_record_clears_it_and_bookkeeping_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let resumed = [
        continued(NEW, "2026-10-16T11:22:00Z"),
        record("assistant", "2026-10-16T12:00:00Z"),
    ];
    assert_eq!(continued_in(&transcript(dir.path(), OLD, &resumed)), None);
    let kept = [
        continued(NEW, "2026-10-16T11:22:00Z"),
        record("mode", "2026-10-16T12:00:00Z"),
    ];
    assert!(continued_in(&transcript(dir.path(), OLD, &kept)).is_some());
}

#[test]
fn only_a_main_transcript_other_than_the_target_counts() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join(OLD).join("subagents");
    fs::create_dir_all(&sub).unwrap();
    let lines = [continued(NEW, "2026-10-16T11:22:00Z")];
    assert_eq!(continued_in(&transcript(&sub, "agent-x", &lines)), None);
    let own = [continued(OLD, "2026-10-16T11:22:00Z")];
    assert_eq!(
        continued_in(&transcript(dir.path(), OLD, &own)),
        None,
        "a record naming itself"
    );
    let side = [format!(
        r#"{{"type":"continued-in","continuedInSessionId":"{NEW}","isSidechain":true}}"#
    )];
    assert_eq!(continued_in(&transcript(dir.path(), OLD, &side)), None);
}

#[test]
fn a_recorded_path_not_named_by_a_session_id_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let got = continued_in(&transcript(
        dir.path(),
        "renamed",
        &[continued(NEW, "2026-10-16T11:22:00Z")],
    ));
    assert_eq!(got.as_ref().map(|s| s.as_str()), Some(NEW));
}

#[test]
fn a_cold_look_reads_only_the_tail_and_complete_lines() {
    let dir = tempfile::tempdir().unwrap();
    // the record, then more than FOLLOW_TAIL of bookkeeping: out of a hook's window, as in Python
    let filler = record("file-history-snapshot", "2026-10-16T11:30:00Z");
    let mut lines = vec![continued(NEW, "2026-10-16T11:22:00Z")];
    lines.extend(std::iter::repeat_n(filler, FOLLOW_TAIL as usize / 100 + 10));
    assert_eq!(continued_in(&transcript(dir.path(), OLD, &lines)), None);
    // a last line without its newline is not read yet
    let path = dir.path().join("partial.jsonl");
    fs::write(&path, continued(NEW, "2026-10-16T11:22:00Z")).unwrap();
    assert_eq!(continued_in(&path), None);
    assert_eq!(continued_in(&dir.path().join("missing.jsonl")), None);
}

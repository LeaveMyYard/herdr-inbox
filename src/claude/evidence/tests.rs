//! The evidence index (Python tests/test_failed_pile.py, review #44 r1-r3,
//! the transcript-level halves).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

const CHECK_ANSWERS: &str = "mcp__plugin_herdr-inbox_herdr-inbox__check_answers";

fn write(path: &Path, records: &[Value]) {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for r in records {
        writeln!(f, "{r}").unwrap();
    }
}

fn tool_use(name: &str, input: Value, id: &str, at: &str) -> Value {
    json!({"type": "assistant", "timestamp": at, "message": {"role": "assistant", "content": [
        {"type": "tool_use", "id": id, "name": name, "input": input}]}})
}

fn tool_result(id: &str, content: &str, at: &str) -> Value {
    json!({"type": "user", "timestamp": at, "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": id, "content": content}]}})
}

fn transcript(records: &[Value]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    write(&path, records);
    (dir, path)
}

const AT: &str = "2026-01-02T10:00:00Z";

#[test]
fn check_answers_output_is_evidence_of_that_exact_answer_only() {
    let body =
        json!({"answers": [{"id": 7, "question": "Short one?", "answer": "yes"}]}).to_string();
    let (_d, p) = transcript(&[
        tool_use(CHECK_ANSWERS, json!({}), "t", AT),
        tool_result("t", &body, AT),
    ]);
    let mut ev = Evidence::default();
    assert!(seen_answer(&mut ev, &p, 7, "  yes ", None)); // whitespace squashed
    assert!(!seen_answer(&mut ev, &p, 7, "yes please", None));
    assert!(!seen_answer(&mut ev, &p, 8, "yes", None));
    // not before the answer's time
    assert!(!seen_answer(
        &mut ev,
        &p,
        7,
        "yes",
        Some("2026-01-02T11:00:00+00:00")
    ));
    assert!(seen_answer(
        &mut ev,
        &p,
        7,
        "yes",
        Some("2026-01-02T10:00:00+00:00")
    ));
}

#[test]
fn the_conversations_own_cli_answer_is_evidence() {
    let cmd =
        "B=~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv-0.10.4/bin/inbox\n$B answer 7 \"x\"";
    let out = "Q7 answered, not delivered: no live agent in pane w1:p4\n";
    let (_d, p) = transcript(&[
        tool_use("Bash", json!({"command": cmd}), "u", AT),
        tool_result("u", out, AT),
    ]);
    assert!(seen_answer(
        &mut Evidence::default(),
        &p,
        7,
        "anything at all",
        None
    ));
}

#[test]
fn r2_a_quoted_cli_line_without_the_own_command_is_not_evidence() {
    let out = "$ inbox answer 7 x\nQ7 answered, pending: busy: the agent is working";
    let cmd = json!({"command": "herdr pane read w1:p6 --source recent"});
    let (_d, p) = transcript(&[tool_use("Bash", cmd, "u1", AT), tool_result("u1", out, AT)]);
    assert!(!seen_answer(
        &mut Evidence::default(),
        &p,
        7,
        "a long answer value here",
        None
    ));
}

#[test]
fn r2_json_the_agent_printed_itself_is_not_evidence() {
    let body = json!({"id": 7, "answer": "yes"}).to_string();
    let cmd = json!({"command": format!("echo '{body}'")});
    let (_d, p) = transcript(&[
        tool_use("Bash", cmd, "u2", AT),
        tool_result("u2", &body, AT),
    ]);
    assert!(!seen_answer(&mut Evidence::default(), &p, 7, "yes", None));
}

#[test]
fn r1_a_result_counts_only_through_the_call_it_answers() {
    let body = json!({"answers": [{"id": 7, "answer": "yes"}]}).to_string();
    let (_d, p) = transcript(&[
        tool_use(CHECK_ANSWERS, json!({}), "t", AT),
        tool_result("other", &body, AT),
    ]);
    assert!(!seen_answer(&mut Evidence::default(), &p, 7, "yes", None));
}

#[test]
fn r1_the_evidence_scan_reads_each_transcript_once() {
    let text = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "x".repeat(200_000)}]}});
    let (_d, p) = transcript(&[text.clone(), text.clone(), text]);
    let mut ev = Evidence::default();
    assert!(!seen_answer(&mut ev, &p, 7, "a long enough answer", None));
    let read = ev.bytes_read;
    assert!(read > 0);
    assert!(!seen_answer(&mut ev, &p, 7, "a long enough answer", None));
    assert_eq!(ev.bytes_read, read); // unchanged file: answered from the index
    // an append is read from the indexed offset
    let body = json!({"answers": [{"id": 7, "answer": "a long enough answer"}]}).to_string();
    write(
        &p,
        &[
            tool_use(CHECK_ANSWERS, json!({}), "t", AT),
            tool_result("t", &body, AT),
        ],
    );
    assert!(seen_answer(&mut ev, &p, 7, "a long enough answer", None));
    assert!(ev.bytes_read - read < 2_000);
}

#[test]
fn r2_a_same_size_rewrite_is_re_indexed() {
    let r1 = json!({"answers": [{"id": 7, "answer": "aaaa"}]}).to_string();
    let (_d, p) = transcript(&[
        tool_use(CHECK_ANSWERS, json!({}), "u3", AT),
        tool_result("u3", &r1, AT),
    ]);
    let mut ev = Evidence::default();
    assert!(seen_answer(&mut ev, &p, 7, "aaaa", None));
    let text = fs::read_to_string(&p).unwrap().replace("aaaa", "bbbb");
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&p, text).unwrap(); // same size, other content
    assert!(!seen_answer(&mut ev, &p, 7, "aaaa", None));
    assert!(seen_answer(&mut ev, &p, 7, "bbbb", None));
}

#[test]
fn a_partial_last_line_waits_until_it_is_whole() {
    let body = json!({"answers": [{"id": 7, "answer": "yes"}]}).to_string();
    let (_d, p) = transcript(&[tool_use(CHECK_ANSWERS, json!({}), "t", AT)]);
    let line = tool_result("t", &body, AT).to_string();
    let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
    write!(f, "{line}").unwrap();
    let mut ev = Evidence::default();
    assert!(!seen_answer(&mut ev, &p, 7, "yes", None));
    writeln!(f).unwrap();
    assert!(seen_answer(&mut ev, &p, 7, "yes", None));
}

#[test]
fn a_boolean_id_is_pythons_int() {
    // Python: isinstance(True, int); the id reads as 1
    let body = json!({"id": true, "answer": "yes"}).to_string();
    let (_d, p) = transcript(&[
        tool_use(CHECK_ANSWERS, json!({}), "t", AT),
        tool_result("t", &body, AT),
    ]);
    assert!(seen_answer(&mut Evidence::default(), &p, 1, "yes", None));
}

#[test]
fn a_missing_transcript_has_no_evidence() {
    assert!(!seen_answer(
        &mut Evidence::default(),
        Path::new("/nonexistent/t.jsonl"),
        7,
        "yes",
        None
    ));
}

/// What `path` holds now, with every `from` replaced, then a filler record.
fn rewritten(path: &Path, swaps: &[(&str, &str)]) -> String {
    let mut text = fs::read_to_string(path).unwrap();
    for (from, to) in swaps {
        text = text.replace(from, to);
    }
    let filler = json!({"type": "user", "timestamp": AT, "message": {"content": "more"}});
    format!("{text}{filler}\n")
}

#[test]
fn a_rewrite_in_place_that_grows_is_read_from_the_start() {
    let r1 = json!({"answers": [{"id": 7, "answer": "aaaa"}]}).to_string();
    let (_d, p) = transcript(&[
        tool_use(CHECK_ANSWERS, json!({}), "u4", AT),
        tool_result("u4", &r1, AT),
    ]);
    let mut ev = Evidence::default();
    assert!(seen_answer(&mut ev, &p, 7, "aaaa", None));
    // the bytes at the indexed edge change too
    let text = rewritten(&p, &[("aaaa", "bbbb"), (AT, "2026-01-03T10:00:00Z")]);
    fs::write(&p, text).unwrap(); // same inode, longer
    assert!(!seen_answer(&mut ev, &p, 7, "aaaa", None));
    assert!(seen_answer(&mut ev, &p, 7, "bbbb", None));
}

#[test]
fn a_replacement_with_the_same_edge_is_read_from_the_start() {
    let r1 = json!({"answers": [{"id": 7, "answer": "aaaa"}]}).to_string();
    let pad = "x".repeat(EDGE as usize * 2); // the answer lies before the edge
    let (d, p) = transcript(&[
        tool_use(CHECK_ANSWERS, json!({}), "u5", AT),
        tool_result("u5", &r1, AT),
        json!({"type": "user", "timestamp": AT, "message": {"content": pad}}),
    ]);
    let mut ev = Evidence::default();
    assert!(seen_answer(&mut ev, &p, 7, "aaaa", None));
    let tmp = d.path().join("new.jsonl");
    fs::write(&tmp, rewritten(&p, &[("aaaa", "bbbb")])).unwrap();
    fs::rename(&tmp, &p).unwrap(); // another inode, longer, the same bytes at the edge
    assert!(!seen_answer(&mut ev, &p, 7, "aaaa", None));
    assert!(seen_answer(&mut ev, &p, 7, "bbbb", None));
}

#[test]
fn the_index_keeps_the_newest_transcripts_only() {
    let d = tempfile::tempdir().unwrap();
    let mut ev = Evidence::default();
    let first = d.path().join("0.jsonl");
    for n in 0..=CACHE_MAX {
        let p = d.path().join(format!("{n}.jsonl"));
        fs::write(&p, "{}\n").unwrap();
        ev.of(&p);
    }
    assert_eq!(ev.cache.len(), CACHE_MAX);
    assert!(!ev.cache.contains_key(first.to_string_lossy().as_ref()));
}

#[test]
fn a_line_with_a_lone_surrogate_is_skipped_as_less_evidence() {
    // a deviation from Python, whose json.loads takes a lone `\ud83d`: less evidence, never more
    let good = json!({"answers": [{"id": 7, "answer": "yes"}]}).to_string();
    let result = tool_result("u6", &good, AT).to_string();
    let (_d, p) = transcript(&[tool_use(CHECK_ANSWERS, json!({}), "u6", AT)]);
    let surrogate = result.replacen('{', "{\"x\":\"\\ud83d\",", 1);
    assert_ne!(surrogate, result);
    let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
    writeln!(f, "{surrogate}").unwrap();
    let mut ev = Evidence::default();
    assert!(!seen_answer(&mut ev, &p, 7, "yes", None));
    writeln!(f, "{result}").unwrap(); // the same line without it is read
    assert!(seen_answer(&mut ev, &p, 7, "yes", None));
}

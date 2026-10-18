use std::fs;

use super::*;

const OWN: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const B: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const C: &str = "aaaaaaaa-0000-4000-8000-000000000003";

fn record(to: &str, ts: Option<&str>, extra: &str) -> String {
    let ts = ts
        .map(|t| format!(r#","timestamp":"{t}""#))
        .unwrap_or_default();
    format!(r#"{{"type":"continued-in","continuedInSessionId":"{to}"{ts}{extra}}}"#)
}

#[test]
fn every_main_continuation_ever_made_once_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{OWN}.jsonl"));
    let lines = [
        record(C, Some("2026-10-16T12:00:00Z"), ""),
        r#"{"type":"user","message":{"content":"resumed"}}"#.to_owned(),
        record(B, Some("2026-10-16T11:00:00Z"), ""),
        record(B, Some("2026-10-16T11:00:00Z"), ""),
        record(B, None, ""),
        record(OWN, None, ""),
        record(C, None, r#","isSidechain":true"#),
        record("../x", None, ""),
        r#"{"type":"progress","note":"continued-in"}"#.to_owned(),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(&path, format!("{text}{}", record(C, None, ""))).unwrap();
    let got: Vec<(String, Option<f64>)> = continued_edges(&path)
        .into_iter()
        .map(|e| (e.target, e.at))
        .collect();
    assert_eq!(
        got,
        [
            (B.to_owned(), None),
            (B.to_owned(), Some(1_792_148_400.0)),
            (C.to_owned(), Some(1_792_152_000.0))
        ],
        "deduplicated, own and sidechain and invalid targets dropped, the partial last line not read"
    );
    let sub = dir.path().join(OWN).join("subagents");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        sub.join("agent-x.jsonl"),
        format!("{}\n", record(B, None, "")),
    )
    .unwrap();
    assert!(continued_edges(&sub.join("agent-x.jsonl")).is_empty());
    assert!(continued_edges(&dir.path().join("missing.jsonl")).is_empty());
}

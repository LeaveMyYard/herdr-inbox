//! Waiting for a typed message: `contains` against Python's answers
//! (tests/fixtures/delivery-messages.json), and the transcript reads on real
//! files: only what was appended after the watched offset counts, every
//! watched transcript is looked at, and a message that never shows gives up
//! at the deadline.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use herdr_inbox::delivery::watch::{contains, landed_since, size, wait_hits};
use serde_json::{Value, json};

const GOLDEN: &str = include_str!("fixtures/delivery-messages.json");
const MESSAGE: &str = "[inbox] Q7 answered — Which region?: eu";

fn append(path: &Path, content: &str) {
    let record = json!({"type": "user", "timestamp": "2026-10-21T10:00:00Z", "message": {"role": "user", "content": content}});
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(f, "{record}").unwrap();
}

#[test]
fn contains_reads_a_record_as_python_does() {
    let doc: Value = serde_json::from_str(GOLDEN).unwrap();
    let rows = doc["contains"].as_array().unwrap();
    assert!(rows.len() >= 14);
    for row in rows {
        let qids: Vec<u64> = row["qids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q.as_u64().unwrap())
            .collect();
        let got = contains(row["text"].as_str().unwrap(), &qids, row["full"].as_str());
        assert_eq!(got, row["contains"].as_bool().unwrap(), "{row}");
    }
}

#[test]
fn only_what_was_appended_after_the_offset_counts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, MESSAGE);
    let offset = size(&path);
    assert!(landed_since(&path, 0, &[7], None));
    assert!(!landed_since(&path, offset, &[7], None)); // it was there before the typing
    append(&path, "draft text then nothing of ours");
    assert!(!landed_since(&path, offset, &[7], Some(MESSAGE)));
    append(&path, &format!("draft text {MESSAGE}"));
    assert!(landed_since(&path, offset, &[7], Some(MESSAGE))); // typed onto a draft
    assert!(!landed_since(&path, offset, &[7], None));
    assert!(!landed_since(
        &dir.path().join("missing.jsonl"),
        0,
        &[7],
        None
    ));
    assert_eq!(size(&dir.path().join("missing.jsonl")), 0);
}

#[test]
fn hits_are_every_watched_transcript_the_message_landed_in() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        dir.path().join("a.jsonl"),
        dir.path().join("b.jsonl"),
        dir.path().join("c.jsonl"),
    );
    for p in [&a, &b, &c] {
        append(p, "earlier");
    }
    let watch = vec![
        (a.clone(), size(&a)),
        (b.clone(), size(&b)),
        (c.clone(), size(&c)),
    ];
    append(&b, MESSAGE);
    append(&c, MESSAGE);
    let hits = wait_hits(&watch, &[7], Some(MESSAGE), Duration::from_secs(5));
    assert_eq!(hits.into_iter().collect::<Vec<_>>(), vec![b, c]);
}

#[test]
fn a_message_that_never_shows_gives_up_at_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    append(&a, "earlier");
    let started = Instant::now();
    let hits = wait_hits(
        &[(a.clone(), size(&a))],
        &[7],
        Some(MESSAGE),
        Duration::from_millis(600),
    );
    let took = started.elapsed();
    assert!(hits.is_empty());
    assert!(
        took >= Duration::from_millis(600) && took < Duration::from_secs(3),
        "{took:?}"
    );
}

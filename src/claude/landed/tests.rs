use std::collections::BTreeMap;

use serde_json::Value;

use super::*;

/// tests/fixtures/landed.json: Python's `transcript.landed_kinds` over generated records.
const GOLDEN: &str = include_str!("../../../tests/fixtures/landed.json");

fn expected_of(golden: &Value) -> BTreeMap<u64, Expected> {
    golden["expected"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, e)| {
            let segments = e["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect();
            (
                id.parse().unwrap(),
                Expected {
                    segments,
                    since: e["since"].as_str().map(str::to_owned),
                },
            )
        })
        .collect()
}

#[test]
fn every_captured_record_lands_as_in_python() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let expected = expected_of(&golden);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let cases = golden["cases"].as_array().unwrap();
    let mut wrong = Vec::new();
    for case in cases {
        let line = case["line"].as_str().unwrap();
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let got: BTreeMap<String, Vec<usize>> = landed_kinds(&path, &expected)
            .into_iter()
            .map(|(id, kinds)| (id.to_string(), kinds.into_iter().collect()))
            .collect();
        let want: BTreeMap<String, Vec<usize>> =
            serde_json::from_value(case["found"].clone()).unwrap();
        if got != want {
            wrong.push(format!("{line}\n  want {want:?}\n  got  {got:?}"));
        }
    }
    assert!(
        cases.len() > 500
            && cases
                .iter()
                .filter(|c| c["found"] != serde_json::json!({}))
                .count()
                > 100
    );
    assert!(
        wrong.is_empty(),
        "{} of {} differ:\n{}",
        wrong.len(),
        cases.len(),
        wrong[..wrong.len().min(5)].join("\n")
    );
}

#[test]
fn only_the_tail_of_a_large_transcript_and_whole_lines_are_read() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let expected = expected_of(&golden);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let hit = r#"{"type":"user","message":{"content":"[inbox] Q7 answered — Which region?: eu"}}"#;
    let filler = format!(
        "{{\"type\":\"progress\",\"data\":\"{}\"}}\n",
        "x".repeat(1000)
    );
    let mut text = format!("{hit}\n");
    text.push_str(&filler.repeat(MAX_READ as usize / filler.len() + 10));
    std::fs::write(&path, &text).unwrap();
    assert!(
        landed_kinds(&path, &expected).is_empty(),
        "beyond MAX_READ from the end"
    );
    text.push_str(hit);
    std::fs::write(&path, &text).unwrap();
    assert_eq!(
        landed_kinds(&path, &expected).keys().collect::<Vec<_>>(),
        [&7],
        "a last line without newline counts"
    );
    assert!(landed_kinds(&dir.path().join("missing"), &expected).is_empty());
}

#[test]
fn a_later_start_is_found_after_any_first_character() {
    assert_eq!(tails("é [inbox] Q1 a"), ["é [inbox] Q1 a", "[inbox] Q1 a"]);
    assert_eq!(
        tails("[inbox] Q1 [inbox] Q2"),
        ["[inbox] Q1 [inbox] Q2", "[inbox] Q2"]
    );
    assert_eq!(
        tails("\n\n<pasted_content id=\"x\">\n [inbox] Q1"),
        ["[inbox] Q1"]
    );
    assert_eq!(
        strip_paste_open(" <pasted_content no end"),
        "<pasted_content no end"
    );
    assert_eq!(tails(""), [""]);
}

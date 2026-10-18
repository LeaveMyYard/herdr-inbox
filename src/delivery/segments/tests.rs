use serde_json::Value;

use super::*;

/// The segments in tests/fixtures/landed.json were written by Python's `service.expected_segments`.
const GOLDEN: &str = include_str!("../../../tests/fixtures/landed.json");

/// The questions of tools/capture_goldens.py's LANDED_QUESTIONS.
fn question(id: i64, question: &str, answer: &str) -> Question {
    Question {
        id,
        created_at: "2026-10-17T10:00:00+00:00".into(),
        session_id: None,
        question: question.into(),
        answer: Some(answer.into()),
        answered_at: Some("2026-10-17T10:00:00+00:00".into()),
        delivery_error: None,
    }
}

#[test]
fn the_segments_are_pythons() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let long = "x".repeat(4100);
    for q in [
        question(7, "Which region?", "eu"),
        question(8, "Squash or merge?\nsecond line", "squash"),
        question(27, "Same as before?", "same as Q28"),
        question(40, "A long one?", &long),
        question(41, "Tabs\tand  spaces?", "a\tb  c\u{2028}d"),
    ] {
        let want: Vec<String> =
            serde_json::from_value(golden["expected"][q.id.to_string()]["segments"].clone())
                .unwrap();
        assert_eq!(expected_segments(&q), want, "Q{}", q.id);
    }
    assert!(
        expected_segments(&question(7, "q", "a"))[REFERENCE_INDEX].contains("call check_answers")
    );
}

#[test]
fn a_null_answer_reads_none() {
    let mut q = question(9, "q?", "");
    q.answer = None;
    assert_eq!(expected_segments(&q)[0], "[inbox] Q9 answered — q?: None");
}

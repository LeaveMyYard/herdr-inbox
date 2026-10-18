use herdr_agent_kit::herdr_api::clean;
use herdr_inbox::delivery::{REFERENCE_INDEX, expected_segments, landed_across};
use herdr_inbox::store::Question;

use crate::common::*;
use crate::transcripts::*;

fn question(id: i64, session: &str, created: &str) -> Question {
    Question {
        id,
        created_at: created.into(),
        session_id: Some(session.into()),
        question: "Which region?".into(),
        answer: Some("eu".into()),
        answered_at: Some("2026-10-17T10:00:00+00:00".into()),
        delivery_error: None,
    }
}

fn typed(q: &Question, i: usize) -> String {
    clean(&expected_segments(q)[i], 1_000_000_000)
}

#[test]
fn a_landing_in_the_old_transcript_before_the_continuation_is_found() {
    // review #41 / Python test_a_hook_answer_that_landed_in_the_old_transcript_is_not_sent_again
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    let q = question(7, OLD, "2026-10-16T10:00:00+00:00");
    p.write(
        OLD,
        &[
            user(OLD, "2026-10-17T10:00:01Z", &typed(&q, 0)),
            continued_in(OLD, NEW, Some("2026-10-17T10:05:00Z")),
        ],
    );
    let got = landed_across(&db, &p.files(), &[q], &[]).unwrap();
    assert_eq!(
        got.get(&7).map(|k| k.iter().copied().collect::<Vec<_>>()),
        Some(vec![0])
    );
}

#[test]
fn a_landing_in_an_ancestor_is_never_a_delivery() {
    // live 2026-10-17 (Q129-Q152): NEW asked, the text landed in OLD, which continued into NEW
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    let q = question(129, NEW, "2026-10-16T12:00:00+00:00");
    p.write(
        OLD,
        &[
            continued_in(OLD, NEW, Some("2026-10-16T11:22:22Z")),
            user(OLD, "2026-10-17T10:00:01Z", &typed(&q, 0)),
        ],
    );
    p.write(NEW, &[user(NEW, "2026-10-16T11:30:00Z", "work")]);
    assert!(
        landed_across(&db, &p.files(), std::slice::from_ref(&q), &[])
            .unwrap()
            .is_empty()
    );
    // ...unless the caller adds that file itself (the hook adds its own payload's transcript)
    let got = landed_across(&db, &p.files(), &[q], &[Some(p.path(OLD)), None]).unwrap();
    assert!(got.contains_key(&129));
}

#[test]
fn a_landing_in_a_forward_continuation_counts_per_question() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let p = Projects::new(dir.path());
    let before = question(1, OLD, "2026-10-16T10:00:00+00:00"); // asked before OLD continued
    let after = question(2, OLD, "2026-10-17T09:00:00+00:00"); // asked on OLD's resumed branch
    p.write(
        OLD,
        &[
            continued_in(OLD, NEW, Some("2026-10-16T11:22:22Z")),
            user(OLD, "2026-10-17T08:00:00Z", "resumed"),
        ],
    );
    let mut reference = question(3, OLD, "2026-10-16T10:00:00+00:00");
    reference.question = "Long?".into();
    p.write(
        NEW,
        &[
            user(NEW, "2026-10-17T10:00:01Z", &typed(&before, 0)),
            user(NEW, "2026-10-17T10:00:01Z", &typed(&after, 0)),
            user(
                NEW,
                "2026-10-17T10:00:01Z",
                &typed(&reference, REFERENCE_INDEX),
            ),
        ],
    );
    let got = landed_across(&db, &p.files(), &[before, after, reference], &[]).unwrap();
    assert!(
        got.contains_key(&1),
        "NEW is in the forward family of a question asked before the move"
    );
    assert!(
        !got.contains_key(&2),
        "never: NEW did not have the branch's context"
    );
    assert_eq!(
        got.get(&3).map(|k| k.iter().copied().collect::<Vec<_>>()),
        Some(vec![REFERENCE_INDEX])
    );
}

use std::time::Duration;

use herdr_inbox::clock::now_minus;
use herdr_inbox::store::{Claim, Store};
use rusqlite::{OptionalExtension, params};

use crate::common::*;

fn store(dir: &std::path::Path) -> (Store, std::path::PathBuf) {
    let path = migrated(dir);
    (
        Store::open(&path, Duration::from_millis(500)).unwrap(),
        path,
    )
}

fn error_of(s: &Store, id: i64) -> Option<String> {
    s.db()
        .query_row(
            "SELECT delivery_error FROM questions WHERE id = ?",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_claim_is_exclusive_until_released_or_stale() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = store(dir.path());
    let q = ask(s.db(), A, "Squash?");
    assert_eq!(
        s.claim_delivery(q).unwrap(),
        None,
        "an open question is not deliverable"
    );
    answer(s.db(), q, "squash");
    let claim = s.claim_delivery(q).unwrap().expect("claimed");
    assert!(
        claim.0.starts_with("sending:")
            && claim.0.len() == "sending:2026-10-17T10:00:00+00:00:".len() + 8
    );
    assert_eq!(error_of(&s, q).as_deref(), Some(claim.0.as_str()));
    assert_eq!(s.claim_delivery(q).unwrap(), None, "held by a live claim");
    s.release_claim(q, &Claim("sending:someone-else".into()), Some("x"))
        .unwrap();
    assert_eq!(
        error_of(&s, q).as_deref(),
        Some(claim.0.as_str()),
        "only our own claim is released"
    );
    s.release_claim(q, &claim, Some("pending: busy")).unwrap();
    assert_eq!(error_of(&s, q).as_deref(), Some("pending: busy"));
    assert!(
        s.claim_delivery(q).unwrap().is_some(),
        "a pending reason is claimable"
    );
    // a claim older than CLAIM_STALE belongs to a process that died mid-send
    let dead = format!("sending:{}:deadbeef", now_minus(Duration::from_secs(61)));
    s.db()
        .execute(
            "UPDATE questions SET delivery_error = ? WHERE id = ?",
            params![dead, q],
        )
        .unwrap();
    assert!(s.claim_delivery(q).unwrap().is_some());
    for (sql, why) in [
        (
            "UPDATE questions SET delivery_error = NULL, delivered_at = 'x' WHERE id = ?",
            "delivered",
        ),
        (
            "UPDATE questions SET delivered_at = NULL, acknowledged_at = 'x' WHERE id = ?",
            "fetched by its agent",
        ),
    ] {
        s.db().execute(sql, [q]).unwrap();
        assert_eq!(s.claim_delivery(q).unwrap(), None, "{why}");
    }
}

#[test]
fn a_hook_send_is_recorded_only_under_our_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = store(dir.path());
    let q = ask(s.db(), A, "Squash?");
    answer(s.db(), q, "squash");
    s.db()
        .execute(
            "INSERT INTO question_ext (id, pending_since) VALUES (?, 'p')",
            [q],
        )
        .unwrap();
    let claim = s.claim_delivery(q).unwrap().unwrap();
    assert!(
        !s.record_hook_delivery(q, &Claim("sending:other".into()))
            .unwrap()
    );
    assert!(s.record_hook_delivery(q, &claim).unwrap());
    let (delivered, error): (Option<String>, Option<String>) = s
        .db()
        .query_row(
            "SELECT delivered_at, delivery_error FROM questions WHERE id = ?",
            [q],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(delivered.is_some() && error.is_none());
    let (sent, pending, group): (Option<String>, Option<String>, Option<i64>) = s
        .db()
        .query_row(
            "SELECT hook_sent_at, pending_since, group_id FROM question_ext WHERE id = ?",
            [q],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(
        sent.is_some() && pending.is_none(),
        "sent, not yet seen; the busy grace is over"
    );
    assert_eq!(group, None, "an existing ext row keeps its group");
    assert!(
        !s.record_hook_delivery(q, &claim).unwrap(),
        "the claim is spent"
    );
    let fresh = ask(s.db(), A, "New?");
    answer(s.db(), fresh, "y");
    let c = s.claim_delivery(fresh).unwrap().unwrap();
    assert!(s.record_hook_delivery(fresh, &c).unwrap());
    let group: Option<i64> = s
        .db()
        .query_row(
            "SELECT group_id FROM question_ext WHERE id = ?",
            [fresh],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(group, Some(fresh), "a new ext row is its own group");
}

#[test]
fn a_verified_landing_is_delivered_seen_and_verified_in_one_write() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = store(dir.path());
    let q = ask(s.db(), A, "Squash?");
    answer(s.db(), q, "squash");
    let claim = s.claim_delivery(q).unwrap().unwrap();
    s.record_delivery(q, None, true, Some(&Claim("sending:other".into())), true)
        .unwrap();
    assert_eq!(
        error_of(&s, q).as_deref(),
        Some(claim.0.as_str()),
        "not our claim: nothing written"
    );
    s.record_delivery(q, None, true, Some(&claim), true)
        .unwrap();
    let row: (Option<String>, Option<String>, Option<String>) = s
        .db()
        .query_row(
            "SELECT delivered_at, acknowledged_at, delivery_error FROM questions WHERE id = ?",
            [q],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(row.0.is_some() && row.1.is_some() && row.2.is_none());
    let verified: Option<String> = s
        .db()
        .query_row(
            "SELECT verified_at FROM question_ext WHERE id = ?",
            [q],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten();
    assert!(verified.is_some());
    // only the reference landed: delivered and verified, not seen (check_answers still returns it)
    let r = ask(s.db(), A, "Long?");
    answer(s.db(), r, "long");
    let c = s.claim_delivery(r).unwrap().unwrap();
    s.record_delivery(r, None, false, Some(&c), true).unwrap();
    let ack: Option<String> = s
        .db()
        .query_row(
            "SELECT acknowledged_at FROM questions WHERE id = ?",
            [r],
            |x| x.get(0),
        )
        .unwrap();
    assert_eq!(ack, None);
    // sent but not seen in the transcript: delivered, never acknowledged nor verified
    let u = ask(s.db(), A, "Unverified?");
    answer(s.db(), u, "u");
    let c = s.claim_delivery(u).unwrap().unwrap();
    s.record_delivery(u, None, true, Some(&c), false).unwrap();
    let row: (Option<String>, Option<String>) = s
        .db()
        .query_row(
            "SELECT delivered_at, acknowledged_at FROM questions WHERE id = ?",
            [u],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap();
    assert!(row.0.is_some() && row.1.is_none(), "{row:?}");
    let verified: Option<Option<String>> = s
        .db()
        .query_row(
            "SELECT verified_at FROM question_ext WHERE id = ?",
            [u],
            |x| x.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(verified.flatten(), None);
}

#[test]
fn a_failed_attempt_is_counted_under_the_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = store(dir.path());
    let q = ask(s.db(), A, "Squash?");
    answer(s.db(), q, "squash");
    let claim = s.claim_delivery(q).unwrap().unwrap();
    s.record_delivery(q, Some("not verified"), true, Some(&claim), false)
        .unwrap();
    let (error, attempts): (Option<String>, i64) = s
        .db()
        .query_row(
            "SELECT delivery_error, delivery_attempts FROM questions WHERE id = ?",
            [q],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((error.as_deref(), attempts), (Some("not verified"), 1));
    s.record_delivery(q, Some("again"), true, Some(&claim), false)
        .unwrap();
    assert_eq!(
        error_of(&s, q).as_deref(),
        Some("not verified"),
        "the claim is spent"
    );
}

#[test]
fn hook_candidates_are_the_familys_carriable_answers_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = store(dir.path());
    let fake = crate::family::Fake::default();
    let q2 = ask(s.db(), A, "Second?");
    let q1 = ask(s.db(), B, "Other?");
    let q3 = ask(s.db(), A, "Third?");
    for q in [q2, q1, q3] {
        answer(s.db(), q, "a");
    }
    s.db()
        .execute(
            "INSERT INTO question_ext (id, hook_misses) VALUES (?, 2)",
            [q3],
        )
        .unwrap();
    let got: Vec<i64> = s
        .hook_candidates(A, 2, &fake)
        .unwrap()
        .iter()
        .map(|q| q.id)
        .collect();
    assert_eq!(got, [q2]);
    let got: Vec<i64> = s
        .hook_candidates(A, 3, &fake)
        .unwrap()
        .iter()
        .map(|q| q.id)
        .collect();
    assert_eq!(got, [q2, q3]);
    let q = &s.hook_candidates(A, 2, &fake).unwrap()[0];
    assert_eq!(
        (
            q.question.as_str(),
            q.answer.as_deref(),
            q.session_id.as_deref()
        ),
        ("Second?", Some("a"), Some(A))
    );
    assert_eq!(q.created_at, "2026-10-17T10:00:00+00:00");
}

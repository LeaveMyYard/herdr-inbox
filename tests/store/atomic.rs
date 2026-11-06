//! Every delivery-state write Python runs in `with self.transaction()` is one
//! transaction here too (#82 final pass L1, like #79's
//! `set_answer_is_all_or_nothing`): a trigger aborts the write's second
//! statement, and the first must be undone, with no transaction left open.

use std::time::Duration;

use herdr_inbox::store::questions::NewQuestion;
use herdr_inbox::store::{Claim, Store};
use serde_json::Value;

use crate::common::migrated;
use crate::golden::{FIXED, tables};

type Prep = fn(&Store, i64) -> Option<Claim>;
type Write = fn(&Store, i64, Option<&Claim>) -> rusqlite::Result<()>;

fn claimed(st: &Store, qid: i64) -> Option<Claim> {
    st.claim_delivery(qid).unwrap()
}

fn nothing(_: &Store, _: i64) -> Option<Claim> {
    None
}

fn sent(st: &Store, qid: i64) -> Option<Claim> {
    st.db()
        .execute(
            "UPDATE question_ext SET hook_sent_at = ? WHERE id = ?",
            rusqlite::params![FIXED, qid],
        )
        .unwrap();
    None
}

/// (name, the table whose write is aborted, setup, the write)
const CASES: [(&str, &str, Prep, Write); 8] = [
    ("set_pending", "question_ext", claimed, |st, q, c| {
        st.set_pending(q, c.unwrap(), "busy: working")
    }),
    ("record_busy_miss", "question_ext", claimed, |st, q, c| {
        st.record_busy_miss(q, c.unwrap(), "busy: unseen")
    }),
    ("hook_verified", "question_ext", nothing, |st, q, _| {
        st.hook_verified(q, true)
    }),
    ("hook_unseen", "questions", sent, |st, q, _| {
        st.hook_unseen(q, FIXED, Some("not seen")).map(drop)
    }),
    ("mark_agent_gone", "questions", nothing, |st, q, _| {
        st.mark_agent_gone(q, "pane closed")
    }),
    ("move_to_pane", "question_ext", nothing, |st, q, _| {
        st.move_to_pane(q, "w1:p9", None, "moved", None, None)
            .map(drop)
    }),
    (
        "mark_seen_elsewhere",
        "question_ext",
        nothing,
        |st, q, _| st.mark_seen_elsewhere(q, None).map(drop),
    ),
    ("mark_landed", "question_ext", nothing, |st, q, _| {
        st.mark_landed(q, true).map(drop)
    }),
];

fn rows(st: &Store) -> Value {
    tables(st, &["questions", "question_ext"])
}

#[test]
fn every_transactional_write_is_all_or_nothing() {
    for (name, aborted, prep, write) in CASES {
        let dir = tempfile::tempdir().unwrap();
        let st = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
        let new = NewQuestion {
            question: "Atomic?",
            session_id: Some("s1"),
            pane_id: Some("w1:p1"),
            ..NewQuestion::default()
        };
        let q = st.insert(&new).unwrap().id;
        st.set_answer(q, "yes", None).unwrap();
        let claim = prep(&st, q);
        let before = rows(&st);
        st.db()
            .execute_batch(&format!(
                "CREATE TEMP TRIGGER abort_ins BEFORE INSERT ON main.{aborted} BEGIN SELECT RAISE(ABORT, 'no'); END;
                 CREATE TEMP TRIGGER abort_upd BEFORE UPDATE ON main.{aborted} BEGIN SELECT RAISE(ABORT, 'no'); END;"
            ))
            .unwrap();
        assert!(
            write(&st, q, claim.as_ref()).is_err(),
            "{name}: the aborted statement is an error"
        );
        assert_eq!(rows(&st), before, "{name}: the first statement was undone");
        assert!(st.db().is_autocommit(), "{name}: no transaction left open");
        st.db()
            .execute_batch("DROP TRIGGER abort_ins; DROP TRIGGER abort_upd;")
            .unwrap();
        write(&st, q, claim.as_ref()).unwrap();
        assert_ne!(
            rows(&st),
            before,
            "{name}: without the trigger the write changes rows"
        );
    }
}

/// No other process may see delivered and acknowledged without `verified_at`
/// (Python test_verified_delivery_is_written_in_one_transaction). The abort
/// is on `verified_at` alone: a trigger on the whole table stops at the
/// earlier `pending_since` update, and never reaches the write this is about.
#[test]
fn a_verified_delivery_is_never_visible_without_verified_at() {
    let dir = tempfile::tempdir().unwrap();
    let st = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    let new = NewQuestion {
        question: "Atomic?",
        session_id: Some("s1"),
        pane_id: Some("w1:p1"),
        ..NewQuestion::default()
    };
    let q = st.insert(&new).unwrap().id;
    st.set_answer(q, "yes", None).unwrap();
    let claim = claimed(&st, q);
    let before = rows(&st);
    st.db()
        .execute_batch(
            "CREATE TEMP TRIGGER abort_verified BEFORE UPDATE OF verified_at ON main.question_ext \
             BEGIN SELECT RAISE(ABORT, 'no'); END;",
        )
        .unwrap();
    assert!(
        st.record_delivery(q, None, true, claim.as_ref(), true)
            .is_err(),
        "the aborted statement is the verified_at write"
    );
    assert_eq!(
        rows(&st),
        before,
        "delivered_at and acknowledged_at went with it"
    );
    assert!(st.db().is_autocommit(), "no transaction left open");
    st.db()
        .execute_batch("DROP TRIGGER abort_verified;")
        .unwrap();
    st.record_delivery(q, None, true, claim.as_ref(), true)
        .unwrap();
    let got = st.get(q).unwrap().unwrap();
    assert!(got.delivered_at.is_some() && got.acknowledged_at.is_some());
    let verified: Option<String> = st
        .db()
        .query_row(
            "SELECT verified_at FROM question_ext WHERE id = ?",
            [q],
            |r| r.get(0),
        )
        .unwrap();
    assert!(verified.is_some());
}

use super::{state, wait};
use inbox_core::deliver::{BUSY_WAIT_IDLE, PANEL};

#[test]
fn the_stored_error_gives_the_state_and_its_prefix_goes() {
    assert_eq!(state(None), ("unattempted", None));
    assert_eq!(
        state(Some("sending:2026-01-02T10:00:00+00:00:abcd1234")),
        ("claimed", None),
        "a claim's token is never published"
    );
    assert_eq!(
        state(Some("pending: busy: x")),
        ("pending", Some("busy: x"))
    );
    assert_eq!(state(Some("agent gone: x")), ("agent_gone", Some("x")));
    assert_eq!(state(Some("no pane")), ("failed", Some("no pane")));
}

/// `busy_idle` and `panel` start like `busy` and `blocked`: the first row that fits decides.
#[test]
fn the_first_wait_word_that_fits_decides() {
    assert_eq!(wait(BUSY_WAIT_IDLE), "busy_idle");
    assert_eq!(wait(PANEL), "panel");
    assert_eq!(wait("busy: x"), "busy");
    assert_eq!(wait("blocked: x"), "blocked");
    assert_eq!(wait("requeued: x"), "requeued");
    assert_eq!(
        wait("waiting: the operator is in that pane"),
        "operator_here"
    );
    assert_eq!(wait("anything"), "other");
}

/// A stranded row keeps its clock in the database, and the feed does not publish it: that
/// field says since when a row waits (the review of #146, N1).
#[test]
fn a_stranded_row_publishes_no_pending_since() {
    use std::time::Duration;

    use crate::store::Store;
    use crate::store::questions::NewQuestion;
    let dir = tempfile::tempdir().unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    let q = store
        .insert(&NewQuestion {
            question: "Question Text 345",
            ..NewQuestion::default()
        })
        .unwrap();
    store.set_answer(q.id, "yes", None).unwrap();
    store
        .db()
        .execute_batch(
            "UPDATE questions SET delivery_error = 'not delivered: the terminal reports error'; \
             UPDATE question_ext SET pending_since = '2020-01-01T00:00:00+00:00';",
        )
        .unwrap();
    let q = store.get(q.id).unwrap().unwrap();
    let row = super::entry(&store, &q, false).unwrap();
    assert_eq!(row["delivery"]["state"], "failed");
    assert!(row["delivery"]["pending_since"].is_null());
    store
        .db()
        .execute_batch("UPDATE questions SET delivery_error = 'pending: busy: the agent is error'")
        .unwrap();
    let q = store.get(q.id).unwrap().unwrap();
    let row = super::entry(&store, &q, false).unwrap();
    assert_eq!(
        row["delivery"]["pending_since"],
        "2020-01-01T00:00:00+00:00"
    );
}

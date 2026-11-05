//! A remote machine's rows through the loop: the mirror read into the model,
//! its effects written, and a refusal after the model moved on (the model's
//! copy kept stale by a read that fails, so the write is what notices).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use super::fake::{Rig, Step, key};
use crate::store::Store;
use crate::tui::mirror_rows::{alert, machine, outbox, question, queued, state};
use crate::tui::screen::Input;
use crate::tui::send::BUSY;

const PAST: &str = "2020-01-01T00:00:00+00:00";

fn rig() -> Rig {
    let rig = Rig::new();
    machine(&rig.store, "m1", Some("box"));
    rig
}

/// A write from another process.
fn sql(rig: &Rig, sql: &'static str) -> Step {
    let db = rig.db.clone();
    Step::Do(Box::new(move || {
        let other = Store::open(&db, Duration::from_secs(5)).unwrap();
        other.db().execute_batch(sql).unwrap();
    }))
}

/// A row the mirror cannot read: every read fails from here on, while writes
/// still work, so the model keeps deciding on its old copy.
const BREAK_READS: &str = "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, \
                           first_seen_at) VALUES ('m1', 'dd', 99, 'k99', 'open', 'not json', 'x');";
const MEND_READS: &str = "DELETE FROM remote_questions WHERE remote_id = 99;";

fn saw(text: &'static str) -> Step {
    Step::Look(Box::new(move |s| assert!(s.contains(text), "{text}:\n{s}")))
}

fn never(text: &'static str) -> Step {
    Step::Look(Box::new(move |s| {
        assert!(!s.contains(text), "{text}:\n{s}")
    }))
}

#[test]
fn the_mirror_shows_and_an_answer_is_queued_under_the_question_shown() {
    let rig = rig();
    question(&rig.store, "m1", 1, 1);
    let steps = vec![
        saw("box:Q1"),
        key("enter"), // open it
        saw("Question Title 1"),
        key("enter"), // option a
        saw("box:Q1 answered · sending to box"),
    ];
    rig.run(None, None, steps);
    let row = rig.store.outbox_rows().unwrap().remove(0);
    assert_eq!((row.action.as_str(), row.qkey.as_str()), ("answer", "k1"));
    assert!(row.detail_json.unwrap().contains("\"choices\": [\"a\"]"));
    let handoff = rig.handoff.calls();
    assert_eq!(
        handoff,
        Vec::<String>::new(),
        "the worker sends it, not the popup"
    );
}

#[test]
fn a_poll_brings_a_new_remote_question() {
    let rig = rig();
    rig.ask("w1:p1", "Local Title 1", &[]);
    rig.ask("w1:p2", "Local Title 2", &[]);
    let db = rig.db.clone();
    let steps = vec![
        never("box:Q7"),
        Step::Do(Box::new(move || {
            let other = Store::open(&db, Duration::from_secs(5)).unwrap();
            question(&other, "m1", 7, 7);
        })),
        Step::Wait(2100),
        Step::Sleep(1),
        saw("box:Q7"),
        Step::Input(Input::Quit),
    ];
    rig.run(Some("w1:p1"), None, steps);
}

#[test]
fn an_answer_refused_after_the_model_moved_on_stays_in_its_view() {
    // a lone row: the model has also closed the popup by the time the write is refused
    let rig = rig();
    question(&rig.store, "m1", 1, 1);
    let steps = vec![
        key("enter"),
        key("down"),
        key("enter"), // "Type something."
        key("z"),
        key("q"),
        key("7"),
        sql(&rig, BREAK_READS),
        sql(
            &rig,
            "UPDATE remote_questions SET qkey = 'other' WHERE remote_id = 1;",
        ),
        key("enter"),
        saw("not queued: that question changed — reopen it"),
        never("answered · sending"), // never two toasts that contradict each other
        saw("Question Title 1"),     // the popup did not close and the view is back
        saw("zq7"),
        sql(&rig, MEND_READS),
        sql(
            &rig,
            "UPDATE remote_questions SET qkey = 'k1' WHERE remote_id = 1;",
        ),
        key("enter"), // the answer is still there:
        key("enter"), // its box, then send: this time it goes
    ];
    rig.run(None, None, steps);
    let row = rig.store.outbox_rows().unwrap();
    assert_eq!(row.len(), 1);
    assert!(
        row[0]
            .detail_json
            .as_deref()
            .unwrap()
            .contains("\"text\": \"zq7\"")
    );
    assert_eq!(row[0].qkey, "k1");
}

#[test]
fn a_busy_database_keeps_the_answer_too() {
    let mut rig = rig();
    rig.timeout = Duration::from_millis(100);
    question(&rig.store, "m1", 1, 1);
    let holder: Rc<RefCell<Option<rusqlite::Connection>>> = Rc::default();
    let (h1, h2, db) = (Rc::clone(&holder), Rc::clone(&holder), rig.db.clone());
    let steps = vec![
        key("enter"),
        Step::Do(Box::new(move || {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("BEGIN EXCLUSIVE").unwrap();
            *h1.borrow_mut() = Some(c);
        })),
        key("enter"),
        saw(BUSY),
        never("answered · sending"),
        saw("Question Title 1"),
        Step::Do(Box::new(move || drop(h2.borrow_mut().take()))),
        key("enter"),
        saw("answered · sending"),
    ];
    rig.run(None, None, steps);
    assert_eq!(queued(&rig.store).len(), 1);
}

#[test]
fn a_dismissal_queues_every_question_of_the_row() {
    let rig = rig();
    question(&rig.store, "m1", 1, 1);
    question(&rig.store, "m1", 2, 1);
    rig.run(
        None,
        None,
        vec![key("d"), key("y"), saw("dismissing on box")],
    );
    let rows = queued(&rig.store);
    let what: Vec<(&str, i64)> = rows.iter().map(|r| (r.0.as_str(), r.1)).collect();
    assert_eq!(what, [("dismiss", 1), ("dismiss", 2)]);
}

#[test]
fn a_dismissal_refused_midway_stops_and_leaves_the_rest_open() {
    let rig = rig();
    question(&rig.store, "m1", 1, 1);
    question(&rig.store, "m1", 2, 1);
    question(&rig.store, "m1", 3, 1);
    let steps = vec![
        key("d"),
        sql(&rig, BREAK_READS),
        sql(
            &rig,
            "UPDATE remote_questions SET qkey = 'other' WHERE remote_id = 2;",
        ),
        key("y"),
        saw("not queued: that question changed — reopen it"),
        never("dismissing on box"),
    ];
    rig.run(None, None, steps);
    let rows = queued(&rig.store);
    let what: Vec<i64> = rows.iter().map(|r| r.1).collect();
    assert_eq!(what, [1], "Q3 is not queued after the refusal");
}

#[test]
fn a_dismissal_refused_midway_never_queues_a_question_twice() {
    // the read fails at the refusal, so the model is back on a copy from before Q1 was queued
    let rig = rig();
    for id in [1, 2, 3] {
        question(&rig.store, "m1", id, 1);
    }
    let dismissed = |rig: &Rig| -> Vec<i64> { queued(&rig.store).iter().map(|r| r.1).collect() };
    let steps = vec![
        key("d"),
        sql(&rig, BREAK_READS),
        sql(
            &rig,
            "UPDATE remote_questions SET qkey = 'other' WHERE remote_id = 2;",
        ),
        key("y"),
        saw("that question changed"),
        sql(
            &rig,
            "UPDATE remote_questions SET qkey = 'k2' WHERE remote_id = 2;",
        ),
        key("d"), // still Q1, Q2 and Q3 as the model sees it
        key("y"),
        saw("that question already has an action queued"),
    ];
    rig.run(None, None, steps);
    assert_eq!(dismissed(&rig), [1]);

    // the reads work again: the row holds what is left, and that goes once each
    rig.store.db().execute_batch(MEND_READS).unwrap();
    rig.run(None, None, vec![key("d"), key("y")]);
    assert_eq!(dismissed(&rig), [1, 2, 3]);
}

#[test]
fn a_refusal_shows_the_database_as_it_is_now() {
    // reads work here: Q1 went out, Q2 failed, and the row says two questions are left
    let rig = rig();
    for id in [1, 2, 3] {
        question(&rig.store, "m1", id, 1);
    }
    rig.store
        .db()
        .execute_batch(
            "CREATE TRIGGER no_q2 BEFORE INSERT ON remote_outbox WHEN NEW.remote_id = 2 \
             BEGIN SELECT RAISE(ABORT, 'no'); END;",
        )
        .unwrap();
    let steps = vec![
        saw("3 questions"),
        key("d"),
        key("y"),
        saw("The inbox database failed"),
        saw("box:Q2"),
        saw("2 questions"),
    ];
    rig.run(None, None, steps);
}

#[test]
fn an_ack_is_written_and_a_refused_one_leaves_the_alert_open() {
    let rig = rig();
    alert(&rig.store, "m1", 4);
    alert(&rig.store, "m1", 5);
    let steps = vec![
        key("down"), // the list starts with no cursor on an alert
        key("d"),    // A4
        saw("acknowledged"),
        Step::Wait(2100),
        Step::Sleep(1),
        sql(&rig, BREAK_READS),
        sql(
            &rig,
            "UPDATE remote_alerts SET akey = 'other' WHERE alert_id = 5;",
        ),
        key("down"),
        key("d"),
        saw("that alert changed or is gone"),
    ];
    rig.run(None, None, steps);
    let rows = queued(&rig.store);
    let what: Vec<(&str, i64)> = rows.iter().map(|r| (r.0.as_str(), r.1)).collect();
    assert_eq!(what, [("ack_alert", 4)]);
    let open: i64 = rig
        .store
        .db()
        .query_row(
            "SELECT COUNT(*) FROM remote_alerts WHERE status = 'open'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open, 1, "A5 stays open");
}

#[test]
fn clearing_an_outbox_row_checks_it_again() {
    let rig = rig();
    question(&rig.store, "m1", 1, 1);
    outbox(&rig.store, "r1", "m1", "queued", PAST);
    outbox(&rig.store, "r2", "m1", "queued", PAST);
    let steps = vec![
        saw("expired · not sent to box"),
        key("d"), // r1: past its expiry, not being sent
        saw("cleared"),
        sql(&rig, BREAK_READS),
        // r2 starts sending: the model's copy still says it can be cleared
        sql(
            &rig,
            "UPDATE remote_outbox SET sending_at = '2027-01-15T08:00:00+00:00' WHERE request_id = 'r2';",
        ),
        key("down"),
        key("d"),
        never("reopen"), // Python says nothing here
        sql(&rig, MEND_READS),
        Step::Wait(2100),
        Step::Sleep(1),
        saw("sending to box now"), // the row is back, as it is
    ];
    rig.run(None, None, steps);
    assert_eq!(
        state(&rig.store, "r1"),
        ("seen".into(), Some("expired".into()))
    );
    assert_eq!(state(&rig.store, "r2"), ("queued".into(), None));
}

#[test]
fn clearing_a_problem_is_written() {
    let rig = rig();
    question(&rig.store, "m1", 9, 9);
    rig.store
        .db()
        .execute_batch(
            "UPDATE remote_questions SET status = 'answered', \
             data_json = '{\"id\": 9, \"group\": 9, \"status\": \"answered\", \"agent_gone\": true, \
             \"question\": \"Question Title 9\", \"answer\": \"Answer 123\"}';",
        )
        .unwrap();
    let steps = vec![saw("box:Q9 answered"), key("d"), saw("cleared")];
    rig.run(None, None, steps);
    assert_eq!(rig.store.problem_rows().unwrap(), []);
}

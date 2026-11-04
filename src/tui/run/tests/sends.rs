//! Background sends under the loop (Python test_async_actions, test_final_review,
//! test_poll_teardown).

use std::sync::Arc;
use std::time::Duration;

use super::fake::{Rig, Step, key};

fn answered(rig: &Rig, qid: i64) -> bool {
    rig.get(qid).status == "answered"
}

/// A row whose answer waits for a busy agent (Python `service.answer` with the agent working).
fn pending(rig: &Rig, qid: i64, error: &str) {
    rig.store
        .set_answer_text(
            qid,
            "yes",
            Some(r#"{"choices": ["yes"], "notes": {}, "text": null}"#),
        )
        .unwrap();
    rig.store
        .db()
        .execute(
            "UPDATE questions SET delivery_error = ? WHERE id = ?",
            rusqlite::params![error, qid],
        )
        .unwrap();
}

#[test]
fn a_lone_answer_is_recorded_handed_off_and_the_popup_closes() {
    // test_popup_records_hands_delivery_off_and_closes_at_once,
    // test_the_worker_is_ensured_before_the_sender_is_spawned
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "Merge?", &["yes", "no"]);
    rig.run(Some("w1:p2"), None, vec![key("enter")]); // the popup closes itself: run returns
    let q = rig.get(qid);
    assert_eq!(
        (q.status.as_str(), q.answer.as_deref()),
        ("answered", Some("yes"))
    );
    assert_eq!(rig.handoff.calls(), ["ensure", "spawn [1] false"]);
    assert_eq!(*rig.marked.lock().unwrap(), [Some("w1:p1".to_string())]);
}

#[test]
fn the_return_pane_is_typed_while_the_popup_holds_the_keyboard() {
    // test_m1_the_return_pane_is_typed_while_the_popup_still_holds_the_keyboard
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "Merge?", &["yes"]);
    rig.run(Some("w1:p1"), None, vec![key("enter")]);
    assert!(answered(&rig, qid));
    assert_eq!(rig.handoff.calls(), ["ensure", "here [1] false"]);
}

#[test]
fn send_now_hands_off_with_force() {
    // test_send_now_hands_off_with_force
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "Late?", &["yes"]);
    rig.ask("w1:p2", "Other?", &[]);
    pending(&rig, qid, "pending: busy: working");
    let steps = vec![
        key("down"),
        key("enter"),
        key("1"),
        key("escape"),
        key("escape"),
    ];
    rig.run(Some("w1:p2"), None, steps);
    assert_eq!(rig.handoff.calls(), ["ensure", "spawn [1] true"]);
}

#[test]
fn send_now_says_when_the_answer_is_already_being_delivered() {
    // test_l3_send_now_says_when_the_answer_is_already_being_delivered
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "Late?", &["yes"]);
    rig.ask("w1:p2", "Other?", &[]);
    pending(&rig, qid, "pending: busy: working");
    let db = rig.db.clone();
    let steps = vec![
        key("down"),
        key("enter"),
        Step::Do(Box::new(move || {
            // another process is sending it right now
            let other = crate::store::Store::open(&db, Duration::from_secs(5)).unwrap();
            assert!(other.claim_delivery(qid).unwrap().is_some());
        })),
        key("1"),
        Step::Sleep(300),
        Step::Wait(1),
        Step::Look(Box::new(|s| {
            assert!(s.contains("Q1 is already being delivered"), "{s}")
        })),
    ];
    rig.run(Some("w1:p2"), None, steps);
    assert!(rig.handoff.calls().is_empty());
}

#[test]
fn closing_waits_for_the_send_in_flight_and_drops_later_keys() {
    // test_closing_the_popup_waits_for_an_in_flight_send; the model takes no event after Quit
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "One?", &["a"]);
    rig.ask("w1:p2", "Other?", &[]);
    rig.handoff.close_gate();
    let gate = Arc::clone(&rig.handoff);
    let opener = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        gate.open_gate();
    });
    let steps = vec![
        key("enter"),  // open the lone question
        key("enter"),  // send "a": back to the list
        key("escape"), // close: waits for the send
        key("enter"),  // dropped
        key("enter"),
    ];
    rig.run(Some("w1:p9"), None, steps);
    // run returned only after the gated delivery step ran
    assert_eq!(rig.handoff.calls(), ["ensure", "spawn [1] false"]);
    assert!(answered(&rig, qid));
    opener.join().unwrap();
}

#[test]
fn the_popup_keeps_taking_keys_while_a_send_is_in_flight() {
    // test_send_runs_in_the_background
    let rig = Rig::new();
    let q1 = rig.ask("w1:p1", "One?", &["a", "b"]);
    rig.ask("w1:p2", "Other?", &[]);
    rig.handoff.close_gate();
    let gate = Arc::clone(&rig.handoff);
    let steps = vec![
        key("enter"), // open the lone question from the list
        key("enter"), // pick "a": sends, back to the list at once
        key("down"),  // the popup keeps reacting while the send is in flight
        Step::Look(Box::new(|s| {
            let row = s.lines().find(|l| l.contains('❯')).unwrap_or("");
            assert!(
                row.contains("Other?"),
                "the cursor moved to the other group:\n{s}"
            );
        })),
        Step::Do(Box::new(move || gate.open_gate())),
        Step::Until(Box::new({
            let h = Arc::clone(&rig.handoff);
            move || h.calls().len() == 2
        })),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), None, steps);
    assert!(answered(&rig, q1));
}

#[test]
fn a_send_reporting_after_the_popup_closed_changes_nothing() {
    // test_a_send_finishing_after_the_popup_exits_does_not_raise: `g` closes the popup
    // while the send is in flight; the send still completes and its report is dropped
    let rig = Rig::new();
    let q1 = rig.ask("w1:p1", "q?", &["yes"]);
    rig.ask("w1:p2", "other?", &[]);
    rig.handoff.close_gate();
    let gate = Arc::clone(&rig.handoff);
    let steps = vec![
        key("enter"),
        key("enter"), // sends, back to the list
        key("down"),
        key("g"), // go to the other agent: the popup closes
        Step::Until(Box::new({
            let a = Arc::clone(&rig.agents);
            move || !a.focused.lock().unwrap().is_empty()
        })),
        Step::Wait(100),
        Step::Do(Box::new(move || gate.open_gate())),
    ];
    rig.run(Some("w1:p1"), None, steps);
    assert!(answered(&rig, q1));
    assert_eq!(rig.handoff.calls().len(), 2);
}

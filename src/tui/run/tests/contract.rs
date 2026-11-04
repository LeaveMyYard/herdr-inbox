//! The loop's side of the model's contract: a fresh read before each key,
//! a refused write ends the action, frames, toasts, quitting, and the marks
//! drained on exit (Python test_tui `test_busy_database_never_crashes_the_popup`,
//! test_async_actions `test_l1_the_popup_drains_background_jobs_before_exit`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use inbox_core::popup::Key;

use super::fake::{Rig, Step, key};
use crate::tui::screen::Input;
use crate::tui::send::BUSY;

#[test]
fn a_key_is_decided_on_the_database_as_it_is_now() {
    // between two polls another process closes Q1: the model hears it before the key, so
    // picking an option says it is no longer open (Python reads the database live)
    let rig = Rig::new();
    let q1 = rig.ask("w1:p1", "one?", &["a", "b"]);
    rig.ask_in("w1:p1", "two?", &["c"], Some(q1));
    let db = rig.db.clone();
    let steps = vec![
        Step::Do(Box::new(move || {
            let other = crate::store::Store::open(&db, Duration::from_secs(5)).unwrap();
            assert!(other.set_status(q1, "withdrawn", None).unwrap());
        })),
        key("enter"), // pick "a": a draft for a question no longer open
        Step::Look(Box::new(|s| {
            assert!(s.contains("Q1 is no longer open"), "{s}")
        })),
        Step::Input(Input::Quit),
    ];
    rig.run(Some("w1:p1"), None, steps);
    assert_eq!(
        rig.get(q1).draft_json,
        None,
        "nothing drafted on a closed question"
    );
}

#[test]
fn a_busy_database_ends_the_action_with_a_notice() {
    let mut rig = Rig::new();
    rig.timeout = Duration::from_millis(200);
    let q1 = rig.ask("w1:p1", "one?", &[]);
    rig.ask("w1:p2", "two?", &[]);
    let holder: Rc<RefCell<Option<rusqlite::Connection>>> = Rc::default();
    let (h1, h2, db) = (Rc::clone(&holder), Rc::clone(&holder), rig.db.clone());
    let steps = vec![
        key("d"), // dismiss the cursor's group? (asks)
        Step::Do(Box::new(move || {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("BEGIN EXCLUSIVE").unwrap();
            *h1.borrow_mut() = Some(c);
        })),
        key("y"),
        Step::Look(Box::new(|s| {
            assert!(s.contains(BUSY), "{s}");
            // the model's copy shows the row closed until the next read (an inert placeholder);
            // no toast reports the dismissal
            assert!(
                !s.contains("Q1 dismissed"),
                "no toast for a write that did not happen:\n{s}"
            );
        })),
        Step::Do(Box::new(move || drop(h2.borrow_mut().take()))),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), None, steps);
    assert_eq!(rig.get(q1).status, "open");
}

#[test]
fn a_dismissal_marks_its_pane_and_exit_waits_for_the_marks() {
    let rig = Rig::new();
    let q1 = rig.ask("w1:p1", "one?", &[]);
    rig.ask("w1:p2", "two?", &[]);
    rig.run(Some("w1:p1"), None, vec![key("d"), key("y"), key("escape")]);
    assert_eq!(rig.get(q1).status, "dismissed");
    assert_eq!(*rig.marked.lock().unwrap(), [Some("w1:p1".to_string())]);
}

#[test]
fn the_empty_inbox_is_drawn_every_frame_and_any_key_closes_it() {
    let rig = Rig::new();
    let steps = vec![Step::Wait(1), Step::Wait(1), Step::Now(Key::Char('x'))];
    let ran = rig.run(None, None, steps);
    let first = &ran.waits[..3];
    assert!(
        first.iter().all(|w| *w <= Duration::from_millis(250)),
        "{first:?}"
    );
}

#[test]
fn the_list_waits_for_the_poll_between_events() {
    let rig = Rig::new();
    rig.ask("w1:p1", "one?", &[]);
    rig.ask("w1:p2", "two?", &[]);
    let ran = rig.run(None, None, vec![Step::Wait(1), Step::Input(Input::Quit)]);
    assert!(
        ran.waits[0] > Duration::from_millis(1500),
        "{:?}",
        ran.waits
    );
}

#[test]
fn ctrl_c_says_how_to_quit_and_its_toast_goes_after_five_seconds() {
    let rig = Rig::new();
    rig.ask("w1:p1", "one?", &[]);
    rig.ask("w1:p2", "two?", &[]);
    let steps = vec![
        Step::Wait(600),
        Step::Input(Input::HelpQuit),
        Step::Look(Box::new(|s| {
            assert!(
                s.contains("Do you want to quit?") && s.contains("Press ctrl+q to quit the app")
            );
        })),
        Step::Wait(5001),
        Step::Wait(1),
        Step::Look(Box::new(|s| assert!(!s.contains("ctrl+q"), "{s}"))),
        Step::Input(Input::Quit), // ctrl+q: closes
        key("enter"),             // dropped
    ];
    rig.run(None, None, steps);
    assert!(rig.handoff.calls().is_empty());
}

#[test]
fn a_gone_terminal_is_not_polled_while_the_sends_finish() {
    // no busy loop: once the terminal is gone the loop only waits for the send
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "one?", &["a"]);
    rig.handoff.close_gate();
    let gate = std::sync::Arc::clone(&rig.handoff);
    let opener = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        gate.open_gate();
    });
    let ran = rig.run(
        Some("w1:p9"),
        None,
        vec![key("enter"), Step::Input(Input::Closed)],
    );
    opener.join().unwrap();
    assert_eq!(
        ran.waits.len(),
        2,
        "the terminal was asked again after it was gone"
    );
    assert_eq!(rig.get(qid).status, "answered");
    assert_eq!(rig.handoff.calls(), ["ensure", "spawn [1] false"]);
}

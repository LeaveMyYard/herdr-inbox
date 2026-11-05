//! Deep links and the popup lock (Python test_deep_link's popup tests).

use std::sync::Arc;

use crate::deeplink::lock::popup_is_running;
use crate::deeplink::request::{unix_now, write_request};
use crate::tui::open::take;
use inbox_core::popup::{DeepLink, Entry};

use super::fake::{Rig, Step, key};

const SESSION: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// Three groups: Q1 (w1:p1); Q2, Q3 (w1:p2, group 2); Q4 (w1:p3).
fn three_groups(rig: &Rig) {
    rig.ask("w1:p1", "first?", &["yes", "no"]);
    let g2 = rig.ask("w1:p2", "second?", &["yes", "no"]);
    rig.ask_in("w1:p2", "third?", &["yes", "no"], Some(g2));
    rig.ask("w1:p3", "fourth?", &["yes", "no"]);
}

fn link(rig: &Rig, kind: &str, value: &str, hold: bool) -> Option<DeepLink> {
    let nonce = write_request(&rig.state, kind, value, hold, unix_now()).unwrap();
    take(&rig.store, &rig.state, Some(&nonce)).1
}

#[test]
fn a_request_names_its_group_for_every_target_kind() {
    for (kind, value, qid) in [
        ("question", "Q3", Some(3)),
        ("group", "2", None),
        ("pane", "w1:p2", None),
        ("session", SESSION, None),
    ] {
        let rig = Rig::new();
        three_groups(&rig);
        rig.store
            .db()
            .execute(
                "UPDATE questions SET session_id = ? WHERE id IN (2, 3)",
                [SESSION],
            )
            .unwrap();
        let l = link(&rig, kind, value, true).unwrap();
        assert_eq!(
            (l.target, l.qid, l.hold),
            (Some(Entry::Group(2)), qid, true),
            "{kind}"
        );
    }
}

#[test]
fn a_held_deep_link_answers_nothing_and_escape_goes_to_the_list() {
    // test_popup_opens_on_the_requested_question, test_without_no_hold_the_popup_stays_held
    let rig = Rig::new();
    three_groups(&rig);
    let l = link(&rig, "question", "3", true);
    let steps = vec![
        key("1"),
        key("enter"),
        key("escape"), // back to the list, not out of the popup
        Step::Look(Box::new(|s| {
            assert!(s.contains("first?") && s.contains("fourth?"), "{s}")
        })),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), l, steps);
    assert_eq!(rig.get(3).status, "open");
    assert!(rig.handoff.calls().is_empty());
}

#[test]
fn a_no_hold_link_opens_like_a_normal_popup() {
    // test_no_hold_opens_like_a_normal_popup
    let rig = Rig::new();
    let qid = rig.ask("w1:p1", "solo?", &["yes", "no"]);
    let l = link(&rig, "question", &qid.to_string(), false);
    rig.run(Some("w1:p2"), l, vec![key("enter")]);
    assert_eq!(rig.get(qid).status, "answered");
}

#[test]
fn a_gone_target_opens_the_list_with_a_notice() {
    // test_gone_target_falls_back_to_the_list_with_a_notice
    let rig = Rig::new();
    three_groups(&rig);
    let l = link(&rig, "question", "9", true);
    assert_eq!(
        l.as_ref().map(|l| l.missing.as_str()),
        Some("Q9 is no longer open")
    );
    let steps = vec![
        Step::Look(Box::new(|s| {
            assert!(s.contains("Q9 is no longer open"), "{s}")
        })),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), l, steps);
}

#[test]
fn a_remote_question_link_is_left_for_now() {
    // remote rows are not in the popup model yet (P6-D2): the popup opens as usual, no notice
    let rig = Rig::new();
    three_groups(&rig);
    assert_eq!(link(&rig, "question", "qnap:Q12", true), None);
}

#[test]
fn the_popup_holds_its_lock_while_it_runs() {
    // test_the_popup_marks_itself_running
    let rig = Rig::new();
    rig.ask("w1:p1", "q?", &[]);
    rig.ask("w1:p2", "q2?", &[]);
    let (lock, _) = take(&rig.store, &rig.state, None);
    let state = Arc::new(rig.state.clone());
    let s2 = Arc::clone(&state);
    let steps = vec![
        Step::Do(Box::new(move || assert!(popup_is_running(&s2)))),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), None, steps);
    drop(lock);
    assert!(!popup_is_running(&state));
}

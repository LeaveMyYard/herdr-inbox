//! `g` under the loop (Python test_async_actions `test_go_to_agent_focuses_off_the_ui_thread`,
//! test_alerts `…g_goes_to_the_pane`).

use std::sync::atomic::Ordering;

use inbox_core::popup::Key;

use super::fake::{Rig, Step, key};

#[test]
fn g_focuses_the_agent_off_the_popups_thread_then_closes() {
    let rig = Rig::new();
    rig.ask("w1:p1", "q1?", &[]);
    rig.ask("w1:p2", "q2?", &[]);
    rig.run(Some("w1:p1"), None, vec![key("g")]); // focused: the popup closes, run returns
    let focused = rig.agents.focused.lock().unwrap().clone();
    assert_eq!(focused, [("w1:p1".to_string(), "inbox-focus".to_string())]);
}

#[test]
fn a_hasty_g_to_an_agent_at_a_prompt_only_says_so() {
    let rig = Rig::new();
    rig.ask("w1:p1", "q1?", &[]);
    rig.ask("w1:p2", "q2?", &[]);
    rig.agents.blocked.store(true, Ordering::SeqCst);
    let steps = vec![
        Step::Now(Key::Char('g')), // right after the view appeared: not deliberate
        Step::Sleep(200),
        Step::Wait(1),
        Step::Look(Box::new(|s| {
            assert!(s.contains("waiting at a prompt"), "{s}")
        })),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), None, steps);
    assert!(rig.agents.focused.lock().unwrap().is_empty());
}

#[test]
fn a_failed_focus_keeps_the_popup_and_says_why() {
    let rig = Rig::new();
    rig.ask("w1:p1", "q1?", &[]);
    rig.ask("w1:p2", "q2?", &[]);
    *rig.agents.error.lock().unwrap() = Some("no such pane".into());
    let steps = vec![
        key("g"),
        Step::Sleep(200),
        Step::Wait(1),
        Step::Look(Box::new(|s| {
            assert!(s.contains("could not focus w1:p1: no such pane"), "{s}");
        })),
        key("escape"),
    ];
    rig.run(Some("w1:p1"), None, steps);
}

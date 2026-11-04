use super::*;

fn q(error: Option<&str>) -> Question {
    Question {
        delivery_error: error.map(String::from),
        ..Question::default()
    }
}

#[test]
fn consecutive_ids_are_a_range_and_others_a_list() {
    assert_eq!(ids(&[5, 3, 4]), "Q3–Q5");
    assert_eq!(ids(&[7, 3]), "Q3, Q7");
    assert_eq!(ids(&[9]), "Q9");
    assert_eq!(ids(&[4, 4]), "Q4, Q4");
}

#[test]
fn the_kind_is_read_from_the_delivery_state() {
    assert_eq!(
        kind(&q(Some("agent gone: no live agent in pane w1:p1"))),
        Kind::Gone
    );
    assert_eq!(
        kind(&q(Some(
            "pending: blocked: the agent is at a prompt or dialog"
        ))),
        Kind::Blocked
    );
    // the task panel is not a prompt the operator answers
    assert_eq!(
        kind(&q(Some(inbox_core::deliver::PANEL)
            .map(|p| format!("pending: {p}"))
            .as_deref())),
        Kind::Stuck
    );
    assert_eq!(
        kind(&q(Some("older conversation: pane w1:p1 runs x"))),
        Kind::Older
    );
    assert_eq!(kind(&q(None)), Kind::Stuck);
}

#[test]
fn the_messages_are_pythons() {
    let (t, b) = message(Kind::Gone, "Q1", "w1:p1");
    assert_eq!(t, "Q1: the agent in w1:p1 is gone");
    assert!(b.contains("check_answers"));
    let (t, b) = message(Kind::Blocked, "Q1", "my-tab");
    assert_eq!(t, "waiting: my-tab is at a prompt (Q1)");
    assert!(!b.contains("check_answers"));
    assert_eq!(
        message(Kind::Stuck, "Q1–Q2", "my-tab").0,
        "Q1–Q2 not delivered to my-tab"
    );
    assert_eq!(
        message(Kind::Older, "Q1", "t").0,
        "Q1: t runs an older conversation"
    );
}

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

/// The four notifications, title and body, character for character Python's.
#[test]
fn the_messages_are_pythons() {
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(
        message(Kind::Gone, "Q1", "w1:p1"),
        pair(
            "Q1: the agent in w1:p1 is gone",
            "The answer is kept; a new session can fetch it with check_answers."
        )
    );
    assert_eq!(
        message(Kind::Older, "Q1", "t"),
        pair(
            "Q1: t runs an older conversation",
            "herdr resumed an earlier session of the asking agent; resume the asking session to \
             deliver (the inbox shows its id), or fetch with check_answers."
        )
    );
    assert_eq!(
        message(Kind::Blocked, "Q1", "my-tab"),
        pair(
            "waiting: my-tab is at a prompt (Q1)",
            "Answer the prompt in that pane; the inbox delivers right after."
        )
    );
    assert_eq!(
        message(Kind::Stuck, "Q1–Q2", "my-tab"),
        pair(
            "Q1–Q2 not delivered to my-tab",
            "The agent can fetch them with check_answers; or retry from the inbox."
        )
    );
}

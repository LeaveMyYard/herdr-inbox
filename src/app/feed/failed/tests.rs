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

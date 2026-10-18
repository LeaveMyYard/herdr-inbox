use super::*;

const S: &str = "aaaaaaaa-0000-4000-8000-000000000001";

fn agent<'a>(session: Option<&'a str>, status: &'a str) -> Option<PaneAgent<'a>> {
    Some(PaneAgent {
        session_id: session,
        status: Some(status),
    })
}

#[test]
fn ready_follows_the_panes_agent() {
    assert_eq!(ready(None, S), Ready::End, "the agent is gone");
    assert_eq!(ready(agent(None, "idle"), S), Ready::Unknown);
    assert_eq!(ready(agent(Some(""), "idle"), S), Ready::Unknown);
    assert_eq!(
        ready(agent(Some("other"), "idle"), S),
        Ready::End,
        "another conversation"
    );
    assert_eq!(ready(agent(Some(S), "idle"), S), Ready::Go);
    assert_eq!(ready(agent(Some(S), "done"), S), Ready::Go);
    for status in ["working", "blocked", "unknown", ""] {
        assert_eq!(ready(agent(Some(S), status), S), Ready::Wait, "{status}");
    }
    assert_eq!(
        ready(
            Some(PaneAgent {
                session_id: Some(S),
                status: None
            }),
            S
        ),
        Ready::Wait
    );
}

#[test]
fn herdr_is_asked_with_answers_pending_or_every_pane_check() {
    let mut w = Watch::new(100.0);
    assert!(!w.look(false, 100.0 + PANE_CHECK_EVERY - 0.1));
    assert!(
        w.look(false, 100.0 + PANE_CHECK_EVERY),
        "a /clear or /resume is noticed"
    );
    assert!(
        !w.look(false, 100.0 + PANE_CHECK_EVERY + 1.0),
        "the clock was reset"
    );
    assert!(w.look(true, 100.0 + PANE_CHECK_EVERY + 1.0));
}

#[test]
fn a_poll_ends_waits_or_delivers() {
    assert_eq!(Watch::owner(false, true), Verdict::End);
    assert_eq!(Watch::owner(true, false), Verdict::End);
    assert_eq!(Watch::owner(true, true), Verdict::KeepWatching);
    let mut w = Watch::new(0.0);
    assert_eq!(
        w.after_look(false, Ready::End, 1.0),
        Verdict::End,
        "with nothing pending too"
    );
    assert_eq!(w.after_look(true, Ready::Go, 1.0), Verdict::Deliver);
    assert_eq!(w.after_look(false, Ready::Go, 1.0), Verdict::KeepWatching);
    assert_eq!(w.after_look(true, Ready::Wait, 1.0), Verdict::KeepWatching);
    assert_eq!(Watch::after_delivery(true), Verdict::Wake);
    assert_eq!(Watch::after_delivery(false), Verdict::KeepWatching);
}

#[test]
fn an_unknown_session_never_wakes_and_ends_the_watch_after_a_while() {
    let mut w = Watch::new(0.0);
    assert_eq!(
        w.after_look(true, Ready::Unknown, 10.0),
        Verdict::KeepWatching
    );
    assert_eq!(
        w.after_look(true, Ready::Unknown, 10.0 + UNKNOWN_SESSION_END - 1.0),
        Verdict::KeepWatching
    );
    // the session reappearing resets the clock
    assert_eq!(
        w.after_look(false, Ready::Wait, 100.0),
        Verdict::KeepWatching
    );
    assert_eq!(
        w.after_look(true, Ready::Unknown, 10.0 + UNKNOWN_SESSION_END + 1.0),
        Verdict::KeepWatching
    );
    assert_eq!(
        w.after_look(true, Ready::Unknown, 11.0 + 2.0 * UNKNOWN_SESSION_END),
        Verdict::End
    );
}

#[test]
fn the_constants_are_pythons() {
    assert_eq!(
        (
            WATCH_POLL,
            PANE_CHECK_EVERY,
            UNKNOWN_SESSION_END,
            LOCK_WAIT,
            WAKE_EXIT
        ),
        (2.0, 45.0, 180.0, 5.0, 3)
    );
    assert_eq!(WATCH_MAX_SECONDS, 42_900.0);
    assert_eq!(Ready::Unknown.name(), "unknown");
    assert_eq!(Verdict::KeepWatching.name(), "keep watching");
}

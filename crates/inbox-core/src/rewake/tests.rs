use super::*;

const S: &str = "aaaaaaaa-0000-4000-8000-000000000001";

fn agent(session: Option<&str>, status: Status) -> Option<PaneAgent<'_>> {
    Some(PaneAgent {
        session_id: session,
        status,
    })
}

#[test]
fn ready_follows_the_panes_agent() {
    assert_eq!(ready(None, S), Ready::End, "the agent is gone");
    assert_eq!(ready(agent(None, Status::Idle), S), Ready::Unknown);
    assert_eq!(ready(agent(Some(""), Status::Idle), S), Ready::Unknown);
    assert_eq!(
        ready(agent(Some("other"), Status::Idle), S),
        Ready::End,
        "another conversation"
    );
    assert_eq!(ready(agent(Some(S), Status::Idle), S), Ready::Go);
    assert_eq!(ready(agent(Some(S), Status::Done), S), Ready::Go);
    for status in [Status::Working, Status::Blocked, Status::Other] {
        assert_eq!(ready(agent(Some(S), status), S), Ready::Wait, "{status:?}");
    }
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
    assert_eq!(Watch::new(0.0).begin_poll(1.0, false, true), Verdict::End);
    assert_eq!(Watch::new(0.0).begin_poll(1.0, true, false), Verdict::End);
    assert_eq!(
        Watch::new(0.0).begin_poll(1.0, true, true),
        Verdict::KeepWatching
    );
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
fn the_watch_ends_at_its_max_age() {
    let w = Watch::new(100.0);
    assert_eq!(
        w.begin_poll(100.0 + WATCH_MAX_SECONDS - 0.5, true, true),
        Verdict::KeepWatching
    );
    assert_eq!(
        w.begin_poll(100.0 + WATCH_MAX_SECONDS, true, true),
        Verdict::End
    );
}

#[test]
fn a_watcher_starts_only_for_a_sessions_own_firing_in_a_pane() {
    assert!(start(true, true, false));
    assert!(!start(false, true, false), "not a canonical session id");
    assert!(!start(true, false, false), "outside herdr");
    assert!(
        !start(true, true, true),
        "a subagent's or teammate's firing"
    );
}

#[test]
fn the_lock_wait_gives_up_after_lock_wait_or_on_losing_ownership() {
    assert!(keep_waiting_for_lock(10.0, 10.0 + LOCK_WAIT - 0.1, true));
    assert!(!keep_waiting_for_lock(10.0, 10.0 + LOCK_WAIT, true));
    assert!(!keep_waiting_for_lock(10.0, 10.5, false));
}

#[test]
fn the_constants_are_pythons() {
    assert_eq!(
        (PANE_CHECK_EVERY, UNKNOWN_SESSION_END, LOCK_WAIT),
        (45.0, 180.0, 5.0)
    );
    assert_eq!(WATCH_MAX_SECONDS, 42_900.0);
    assert_eq!(Ready::Unknown.name(), "unknown");
    assert_eq!(Verdict::KeepWatching.name(), "keep watching");
}

#[test]
fn a_go_verdict_also_resets_the_unknown_session_clock() {
    let mut w = Watch::new(0.0);
    assert_eq!(
        w.after_look(false, Ready::Unknown, 10.0),
        Verdict::KeepWatching
    );
    assert_eq!(w.after_look(false, Ready::Go, 100.0), Verdict::KeepWatching);
    // without the reset the clock would date from 10.0 and end here
    assert_eq!(
        w.after_look(false, Ready::Unknown, 11.0 + UNKNOWN_SESSION_END),
        Verdict::KeepWatching
    );
}

use std::time::Duration;

use super::{ABSENT_EXIT, Idle, NO_PROOF_EXIT, State, state};

#[test]
fn only_30_seconds_of_confirmed_absence_ends_the_writer() {
    let mut idle = Idle::default();
    assert!(!idle.observe(State::Absent, Duration::from_secs(100)));
    assert!(!idle.observe(State::Absent, Duration::from_secs(129)));
    assert!(idle.observe(State::Absent, Duration::from_secs(100) + ABSENT_EXIT));
}

#[test]
fn anything_else_restarts_the_count() {
    for state in [State::Running, State::Unknown] {
        let mut idle = Idle::default();
        idle.observe(State::Absent, Duration::from_secs(0));
        assert!(!idle.observe(state, Duration::from_secs(29)));
        assert!(
            !idle.observe(State::Absent, Duration::from_secs(31)),
            "{state:?}"
        );
        assert!(
            !idle.observe(State::Absent, Duration::from_secs(60)),
            "{state:?}"
        );
        assert!(
            idle.observe(State::Absent, Duration::from_secs(61)),
            "{state:?}"
        );
    }
}

#[test]
fn only_exit_0_and_exit_2_say_anything() {
    assert_eq!(state(Some(0)), State::Running);
    assert_eq!(state(Some(2)), State::Absent);
    for says_nothing in [Some(1), Some(3), Some(127), None] {
        assert_eq!(state(says_nothing), State::Unknown);
    }
}

/// With no `herdr-app` on `PATH` every look is `Unknown`, which restarts the absence count.
#[test]
fn ten_minutes_with_no_proof_end_the_writer_and_a_running_app_keeps_it() {
    let mut idle = Idle::default();
    assert!(!idle.observe(State::Unknown, Duration::from_secs(5)));
    assert!(!idle.observe(
        State::Unknown,
        Duration::from_secs(5) + NO_PROOF_EXIT - Duration::from_secs(1)
    ));
    assert!(idle.observe(State::Unknown, Duration::from_secs(5) + NO_PROOF_EXIT));

    let mut idle = Idle::default();
    idle.observe(State::Running, Duration::from_secs(0));
    assert!(!idle.observe(State::Running, NO_PROOF_EXIT));
    assert!(!idle.observe(State::Unknown, NO_PROOF_EXIT + Duration::from_secs(1)));
}

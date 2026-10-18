use std::collections::BTreeSet;

use super::*;

#[test]
fn only_the_asking_sessions_own_hook_carries() {
    assert!(own_session(Some("s"), false));
    assert!(
        !own_session(Some("s"), true),
        "a subagent's or teammate's hook"
    );
    assert!(!own_session(Some(""), false));
    assert!(!own_session(None, false));
}

#[test]
fn each_candidate_is_skipped_recorded_released_or_sent() {
    let answer: BTreeSet<usize> = [0].into();
    let reference: BTreeSet<usize> = [REFERENCE_INDEX].into();
    let both: BTreeSet<usize> = [1, REFERENCE_INDEX].into();
    let c = |claimed, landed, in_this_message| Candidate {
        claimed,
        landed,
        in_this_message,
    };
    assert_eq!(
        carry(&c(false, Some(&answer), true)),
        Carry::NotClaimed,
        "the claim decides first"
    );
    assert_eq!(
        carry(&c(true, Some(&answer), false)),
        Carry::Landed { acknowledge: true }
    );
    assert_eq!(
        carry(&c(true, Some(&reference), true)),
        Carry::Landed { acknowledge: false }
    );
    assert_eq!(
        carry(&c(true, Some(&both), true)),
        Carry::Landed { acknowledge: true }
    );
    assert_eq!(carry(&c(true, None, false)), Carry::NextFiring);
    assert_eq!(carry(&c(true, None, true)), Carry::Send);
}

#[test]
fn the_message_follows_what_was_recorded() {
    assert_eq!(message(0, 2), Message::Nothing);
    assert_eq!(message(2, 2), Message::AsBuilt);
    assert_eq!(message(1, 2), Message::Rebuilt);
}

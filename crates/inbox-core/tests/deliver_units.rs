//! What the decision table cannot pin, because the recorder masks session
//! ids and keeps no ids in an agent view (#92 review M1, L1, N1): which
//! session each error names, a retry against a pane that switched
//! conversations, and Python's truthiness. Expected texts are Python
//! `service.older_error`'s output for the same arguments.

use inbox_core::deliver::{
    Agent, Composer, Landing, Look, Mode, NOT_VERIFIED, OPERATOR_IN_PANE, Outcome, Waiting, decide,
    older_error,
};

const ASKER: &str = "aaaaaaaa-2222-4000-8000-00000000000a";
const LIVE: &str = "bbbbbbbb-1111-4000-8000-00000000000b";
const REPORTED: &str = "cccccccc-3333-4000-8000-00000000000c";
const OTHER: &str = "dddddddd-4444-4000-8000-00000000000d";

fn agent(session: &str) -> Agent {
    Agent {
        kind: Some("claude".into()),
        status: "idle".into(),
        focused: false,
        session: Some(session.into()),
        reported_session: Some(REPORTED.into()),
    }
}

/// An idle pane where every gate lets the answer through, unless a field says otherwise.
struct Fake {
    changed: bool,
    runs_older: bool,
    reports_another: bool,
    typed_older: bool,
    again: Option<Agent>,
    prompt_error: Option<String>,
    landing: Landing,
    prompted: usize,
}

impl Default for Fake {
    fn default() -> Fake {
        Fake {
            changed: false,
            runs_older: false,
            reports_another: false,
            typed_older: false,
            again: Some(agent(LIVE)),
            prompt_error: None,
            landing: Landing {
                counted: true,
                anywhere: true,
                older_error: None,
            },
            prompted: 0,
        }
    }
}

impl Look for Fake {
    fn claimed(&mut self, _: i64) -> bool {
        true
    }
    fn agent(&mut self) -> Option<Agent> {
        Some(agent(LIVE))
    }
    fn moved_elsewhere(&mut self, _: i64) -> bool {
        false
    }
    fn conversation_changed(&mut self, _: i64) -> bool {
        self.changed
    }
    fn pane_runs_older_session(&mut self, _: i64) -> bool {
        self.runs_older
    }
    fn watched(&mut self) -> bool {
        false
    }
    fn hook_misses(&mut self, _: i64) -> i64 {
        0
    }
    fn hook_live(&mut self) -> bool {
        false
    }
    fn busy_attempts(&mut self, _: i64) -> i64 {
        0
    }
    fn within_busy_grace(&mut self, _: i64) -> bool {
        false
    }
    fn panel_focused(&mut self) -> bool {
        false
    }
    fn landed(&mut self, ids: &[i64]) -> Vec<Option<Vec<usize>>> {
        vec![None; ids.len()]
    }
    fn pane_reports_another_session(&mut self) -> bool {
        self.reports_another
    }
    fn already_typed_into_older_session(&mut self, _: i64) -> bool {
        self.typed_older
    }
    fn messages(&mut self, ids: &[i64]) -> Vec<(Vec<i64>, bool)> {
        vec![(ids.to_vec(), true)]
    }
    fn agent_again(&mut self, _: &[i64]) -> Option<Agent> {
        self.again.clone()
    }
    fn composer(&mut self, _: &[i64]) -> Composer {
        Composer::Empty
    }
    fn prompt(&mut self, _: &[i64]) -> Option<String> {
        self.prompted += 1;
        self.prompt_error.clone()
    }
    fn verify_transcript(&mut self, _: &[i64]) -> bool {
        true
    }
    fn landing(&mut self, _: i64) -> Landing {
        self.landing.clone()
    }
}

fn one(look: &mut Fake, retry: bool) -> Outcome {
    let waiting = [Waiting {
        id: 1,
        session: Some(ASKER.into()),
        requeued: false,
    }];
    let mode = Mode {
        retry,
        ..Mode::default()
    };
    let [(_, outcome)]: [(i64, Outcome); 1] = decide(Some("w3:p9"), &waiting, mode, look)
        .try_into()
        .unwrap();
    outcome
}

#[test]
fn older_error_names_the_pane_session_then_the_asker_as_python_does() {
    assert_eq!(
        older_error("w3:p9", Some(LIVE), Some(ASKER), false),
        "older conversation: pane w3:p9 runs bbbbbbbb, an older session than the asking aaaaaaaa; resume aaaaaaaa to deliver"
    );
    assert_eq!(
        older_error("w3:p9", Some(LIVE), Some(ASKER), true),
        "older conversation: typed into bbbbbbbb in pane w3:p9, an older session than the asking aaaaaaaa, which never got it; resume aaaaaaaa to deliver"
    );
    // Python `(live or "?")`: empty reads as missing
    assert_eq!(
        older_error("w3:p9", Some(""), Some(ASKER), false),
        "older conversation: pane w3:p9 runs ?, an older session than the asking aaaaaaaa; resume aaaaaaaa to deliver"
    );
    assert_eq!(
        older_error("w3:p9", None, Some(""), true),
        "older conversation: typed into ? in pane w3:p9, an older session than the asking ?, which never got it; resume ? to deliver"
    );
}

#[test]
fn a_pane_running_an_older_session_names_the_followed_session() {
    let mut look = Fake {
        changed: true,
        runs_older: true,
        ..Fake::default()
    };
    assert_eq!(
        one(&mut look, false),
        Outcome::Failed(older_error("w3:p9", Some(LIVE), Some(ASKER), false))
    );
}

#[test]
fn an_answer_already_typed_into_an_older_session_names_the_reported_session() {
    let mut look = Fake {
        reports_another: true,
        typed_older: true,
        ..Fake::default()
    };
    assert_eq!(
        one(&mut look, false),
        Outcome::Failed(older_error("w3:p9", Some(REPORTED), Some(ASKER), true))
    );
    assert_eq!(look.prompted, 0);
}

#[test]
fn a_retry_never_types_into_a_conversation_that_replaced_the_asker() {
    let mut look = Fake {
        again: Some(agent(OTHER)),
        ..Fake::default()
    };
    assert_eq!(
        one(&mut look, true),
        Outcome::Pending(OPERATOR_IN_PANE.into())
    );
    assert_eq!(look.prompted, 0);
    let mut same = Fake::default();
    assert_eq!(one(&mut same, true), Outcome::Delivered { ack: true });
    assert_eq!(same.prompted, 1);
}

#[test]
fn an_empty_prompt_error_is_a_success_as_pythons_if_err() {
    let mut look = Fake {
        prompt_error: Some(String::new()),
        ..Fake::default()
    };
    assert_eq!(one(&mut look, false), Outcome::Delivered { ack: true });
}

#[test]
fn an_empty_older_state_is_no_older_landing() {
    let landing = Landing {
        counted: false,
        anywhere: true,
        older_error: Some(String::new()),
    };
    let mut look = Fake {
        landing,
        ..Fake::default()
    };
    assert_eq!(one(&mut look, false), Outcome::Failed(NOT_VERIFIED.into()));
}

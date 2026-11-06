use std::sync::Mutex;

use herdr_agent_kit::agent_procs::fake::FakeProcesses;

use super::fake::{FakeHc, GET_17};
use super::*;
use crate::host::tui::Tui;
use crate::mcp::caller::{Pane as HerdrPane, World};

const ENV_SESSION: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const RECORDED: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// The world: `claude` pid 20 under a shell, its start token as `starts` says, read in turn.
struct Fake {
    starts: Mutex<Vec<&'static str>>,
    record: Record,
    /// herdr's foreground of the pane.
    pane: Option<Vec<Pid>>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            starts: Mutex::new(vec!["s20", "s20"]),
            record: Record::Missing,
            pane: None,
        }
    }
}

impl Machine for Fake {
    type Table = FakeProcesses;
    fn table(&self) -> FakeProcesses {
        let mut starts = self.starts.lock().unwrap();
        let start = if starts.len() > 1 {
            starts.remove(0)
        } else {
            starts[0]
        };
        let mut t = FakeProcesses::default();
        t.insert(19, 1, "zsh", "s19", &["zsh"]);
        t.insert(20, 19, "claude", start, &["claude"]);
        t
    }
    fn parent(&self) -> Pid {
        20
    }
    fn recorded_session(&self, pid: Pid, start: &str) -> Record {
        assert_eq!((pid, start), (20, "s20"));
        self.record.clone()
    }
    fn pane_foreground(&self, _: &str) -> Option<Vec<Pid>> {
        self.pane.clone()
    }
    fn live(&self, reported: &str) -> String {
        format!("live {reported}")
    }
}

fn core(claim: Option<&str>, machine: Fake) -> Core<FakeHc, Fake> {
    Core {
        claim: claim.map(str::to_owned),
        hc: FakeHc::recorded(GET_17),
        machine,
    }
}

fn env() -> Env {
    Env {
        pane_id: Some("w1:p1".into()), // inherited: a proved terminal goes first
        session_id: Some(ENV_SESSION.into()),
        tab_id: Some("w1:t1".into()),
        cwd: Some("/r".into()),
    }
}

#[test]
fn a_proved_caller_has_a_seat_and_no_pane() {
    let c = core(Some("t2"), Fake::new()).caller(&env()).unwrap();
    assert_eq!(
        c,
        Some(Caller {
            session_id: Some(format!("live {ENV_SESSION}")),
            cwd: Some("/r".into()),
            core: Some(CoreSeat {
                terminal: "t2".into(),
                machine: None
            }),
            ..Caller::default()
        })
    );
}

#[test]
fn claude_codes_own_record_beats_the_environment() {
    // `/clear` rewrites the record; CLAUDE_CODE_SESSION_ID stays as it was at start
    let machine = Fake {
        record: Record::Names(RECORDED.into()),
        ..Fake::new()
    };
    let c = core(Some("t2"), machine).caller(&env()).unwrap().unwrap();
    assert_eq!(c.session_id, Some(format!("live {RECORDED}")));
}

fn unproved(result: Result<Option<Caller>, Failure>) -> bool {
    matches!(result, Err(Failure::Unproved))
}

#[test]
fn no_session_is_unproved() {
    for session in [None, Some(String::new())] {
        let e = Env {
            session_id: session,
            ..env()
        };
        assert!(unproved(core(Some("t2"), Fake::new()).caller(&e)));
    }
}

#[test]
fn a_record_that_cannot_be_used_refuses_with_no_fallback_to_the_environment() {
    let machine = Fake {
        record: Record::Unusable,
        ..Fake::new()
    };
    assert!(unproved(core(Some("t2"), machine).caller(&env())));
}

#[test]
fn a_process_replaced_during_the_proof_is_unproved() {
    let machine = Fake {
        starts: Mutex::new(vec!["s20", "other"]),
        ..Fake::new()
    };
    assert!(unproved(core(Some("t2"), machine).caller(&env())));
}

#[test]
fn no_claim_asks_nothing() {
    for claim in [None, Some("")] {
        let c = core(claim, Fake::new());
        assert_eq!(c.caller(&env()).unwrap(), None);
        assert!(c.hc.asked.lock().unwrap().is_empty());
    }
}

/// herdr's pane w9:p9, whose agent runs another conversation: the real `resolve` adopts it.
struct OtherPane;

impl World for OtherPane {
    fn nested(&self) -> bool {
        false
    }
    fn pane(&self, _: &str) -> Option<HerdrPane> {
        Some(HerdrPane {
            tab_id: Some("w9:t9".into()),
            session: Some(RECORDED.into()),
        })
    }
    fn tab_label(&self, _: &str) -> Option<String> {
        None
    }
    fn agents(&self) -> Vec<(String, Option<String>)> {
        Vec::new()
    }
    fn live(&self, reported: &str, _: Option<&str>) -> String {
        reported.to_owned()
    }
    fn marks(&self, _: Option<&str>) -> Result<Vec<String>, Failure> {
        Ok(Vec::new())
    }
}

fn hosts(claim: &str, machine: Fake) -> Vec<Box<dyn Host>> {
    vec![
        Box::new(core(Some(claim), machine)),
        Box::new(Tui {
            world: Box::new(OtherPane),
        }),
    ]
}

#[test]
fn a_refused_claim_never_takes_the_pane_it_inherited() {
    // the review's Q6 and Q8: any refusal, with HERDR_PANE_ID inherited from the core
    for pane in [None, Some(vec![]), Some(vec![77])] {
        let machine = Fake {
            pane,
            ..Fake::new()
        };
        let got = crate::host::caller(&hosts("t9", machine), &env());
        assert!(matches!(got, Err(Failure::Unproved)), "{got:?}");
    }
}

#[test]
fn a_refused_claim_with_no_pane_is_unproved_too() {
    let e = Env {
        pane_id: None,
        ..env()
    };
    assert!(unproved(core(Some("t9"), Fake::new()).caller(&e)));
}

#[test]
fn a_pane_that_herdr_says_holds_this_claude_stays_a_pane() {
    // a herdr TUI started inside a core terminal
    let machine = Fake {
        pane: Some(vec![19, 20]),
        ..Fake::new()
    };
    let c = crate::host::caller(&hosts("t9", machine), &env()).unwrap();
    assert_eq!(c.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(c.session_id.as_deref(), Some(RECORDED));
    assert_eq!(c.core, None);
}

#[test]
fn a_pane_proof_needs_the_process_still_there() {
    let machine = Fake {
        pane: Some(vec![20]),
        starts: Mutex::new(vec!["s20", "s20", "other"]),
        ..Fake::new()
    };
    assert!(unproved(core(Some("t9"), machine).caller(&env())));
}

#[test]
fn a_proved_terminal_goes_before_an_inherited_pane() {
    let c = crate::host::caller(&hosts("t2", Fake::new()), &env()).unwrap();
    assert_eq!(c.pane_id, None);
    assert!(c.core.is_some());
}

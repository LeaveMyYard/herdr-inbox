use std::sync::Mutex;

use herdr_agent_kit::agent_procs::fake::FakeProcesses;

use super::fake::{FakeHc, GET_17, LS_17};
use super::*;

const ENV_SESSION: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const RECORDED: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// The world: `claude` pid 20 under a shell, its start token as `starts` says, read in turn.
struct Fake {
    starts: Mutex<Vec<&'static str>>,
    record: Option<&'static str>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            starts: Mutex::new(vec!["s20", "s20"]),
            record: None,
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
    fn recorded_session(&self, pid: Pid, start: &str) -> Option<String> {
        assert_eq!((pid, start), (20, "s20"));
        self.record.map(str::to_owned)
    }
    fn live(&self, reported: &str) -> String {
        format!("live {reported}")
    }
}

fn core(claim: Option<&str>, machine: Fake) -> Core<FakeHc, Fake> {
    Core {
        claim: claim.map(str::to_owned),
        hc: FakeHc::recorded(GET_17, LS_17),
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
        record: Some(RECORDED),
        ..Fake::new()
    };
    let c = core(Some("t2"), machine).caller(&env()).unwrap().unwrap();
    assert_eq!(c.session_id, Some(format!("live {RECORDED}")));
}

#[test]
fn no_session_is_no_caller() {
    let e = Env {
        session_id: None,
        ..env()
    };
    assert_eq!(core(Some("t2"), Fake::new()).caller(&e).unwrap(), None);
    let e = Env {
        session_id: Some(String::new()),
        ..env()
    };
    assert_eq!(core(Some("t2"), Fake::new()).caller(&e).unwrap(), None);
}

#[test]
fn a_process_replaced_during_the_proof_is_no_caller() {
    let machine = Fake {
        starts: Mutex::new(vec!["s20", "other"]),
        record: None,
    };
    assert_eq!(core(Some("t2"), machine).caller(&env()).unwrap(), None);
}

#[test]
fn no_claim_asks_nothing() {
    for claim in [None, Some("")] {
        let c = core(claim, Fake::new());
        assert_eq!(c.caller(&env()).unwrap(), None);
        assert!(c.hc.asked.lock().unwrap().is_empty());
    }
}

#[test]
fn a_refused_claim_is_no_caller_of_this_host() {
    assert_eq!(core(Some("t9"), Fake::new()).caller(&env()).unwrap(), None);
}

/// A pane's caller, whatever is asked.
struct Pane;

impl Host for Pane {
    fn caller(&self, _: &Env) -> Result<Option<Caller>, Failure> {
        Ok(Some(Caller {
            pane_id: Some("w1:p1".into()),
            ..Caller::default()
        }))
    }
}

#[test]
fn a_spoofed_claim_in_a_pane_leaves_the_pane_what_it_was() {
    let hosts: Vec<Box<dyn Host>> = vec![Box::new(core(Some("t9"), Fake::new())), Box::new(Pane)];
    let c = crate::host::caller(&hosts, &env()).unwrap();
    assert_eq!(c.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(c.core, None);
}

#[test]
fn a_proved_terminal_goes_before_an_inherited_pane() {
    let hosts: Vec<Box<dyn Host>> = vec![Box::new(core(Some("t2"), Fake::new())), Box::new(Pane)];
    let c = crate::host::caller(&hosts, &env()).unwrap();
    assert_eq!(c.pane_id, None);
    assert!(c.core.is_some());
}

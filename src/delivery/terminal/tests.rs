use std::sync::Mutex;

use herdr_agent_kit::agent_procs::fake::FakeProcesses;
use serde_json::json;

use super::*;
use crate::host::core::fake::{FakeHc, GET_17, GET_18};

mod table;

const SESSION: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// `claude` pid 20 under a shell, as `get-0.1.17.json` says.
struct Box20 {
    record: Mutex<Vec<Record>>,
    claude: bool,
}

impl Box20 {
    fn new(record: Record) -> Box20 {
        Box20 {
            record: Mutex::new(vec![record]),
            claude: true,
        }
    }
}

impl Machine for Box20 {
    type Table = FakeProcesses;
    fn table(&self) -> FakeProcesses {
        let mut t = FakeProcesses::default();
        t.insert(19, 1, "zsh", "s19", &["zsh"]);
        if self.claude {
            t.insert(20, 19, "claude", "s20", &["claude"]);
        } else {
            t.insert(20, 19, "zsh", "s20", &["claude"]);
        }
        t
    }
    fn parent(&self) -> Pid {
        1
    }
    fn recorded_session(&self, pid: Pid, start: &str) -> Record {
        assert_eq!((pid, start), (20, "s20"));
        self.record.lock().unwrap()[0].clone()
    }
    fn live(&self, _: &str) -> String {
        panic!("a terminal's conversation is the record's: it is never followed")
    }
    fn pane_foreground(&self, _: &str) -> Option<Vec<Pid>> {
        None
    }
}

fn hosted(get: &str, record: Record) -> Hosted<FakeHc, Box20> {
    Hosted {
        hc: FakeHc::recorded(get),
        machine: Box20::new(record),
    }
}

#[test]
fn a_recorded_claude_is_seen_with_its_status_and_session() {
    let terminals = hosted(GET_17, Record::Names(SESSION.into()));
    let Seen::Here(seated) = terminals.see("t2") else {
        panic!("not seen");
    };
    assert_eq!(seated.agent.status, "working");
    assert_eq!(seated.agent.session.as_deref(), Some(SESSION));
    assert_eq!(seated.agent.reported_session.as_deref(), Some(SESSION));
    assert_eq!(seated.agent.kind.as_deref(), Some("claude"));
    assert!(!seated.agent.focused);
    assert_eq!((seated.pid, seated.start.as_str()), (20, "s20"));
}

#[test]
fn the_status_word_is_read_from_every_recorded_shape() {
    assert_eq!(status(&json!({"status": "working"})), "working");
    assert_eq!(
        status(&json!({"status": {"app": "claude-code", "state": "done"}})),
        "done"
    );
    assert_eq!(status(&json!({"status": null})), "None");
    assert_eq!(status(&json!({})), "None");
    let recorded: Value = serde_json::from_str(GET_18).unwrap();
    assert_eq!(status(&recorded), "None");
}

#[test]
fn a_process_at_that_pid_that_is_not_the_foreground_program_is_never_taken_for_it() {
    let mut terminals = hosted(GET_17, Record::Names(SESSION.into()));
    terminals.machine.claude = false;
    assert_eq!(terminals.see("t2"), Seen::Unread(REPLACED.into()));
}

#[test]
fn an_answer_for_another_address_is_no_answer() {
    let mut fake = FakeHc::recorded(GET_17);
    fake.terminal("t2")["state"] = json!("finished");
    let terminals = Hosted {
        hc: fake,
        machine: Box20::new(Record::Missing),
    };
    assert_eq!(terminals.see("t2"), Seen::Gone);
    let read = json!({"address": "t9", "state": "live"});
    assert!(matches!(gone_or_other(&read, "t2"), Seen::Unread(_)));
}

#[test]
fn a_foreground_not_sampled_yet_says_nothing_about_the_terminal() {
    let mut fake = FakeHc::recorded(GET_17);
    fake.terminal("t2")["foreground"] = Value::Null;
    let terminals = Hosted {
        hc: fake,
        machine: Box20::new(Record::Missing),
    };
    assert!(matches!(terminals.see("t2"), Seen::Unread(_)));
}

#[test]
fn no_core_reads_as_nothing_and_types_nothing() {
    assert!(matches!(NoCore.see("t1"), Seen::Unread(_)));
    assert_eq!(NoCore.screen("t1"), None);
    assert!(matches!(NoCore.send("t1", "x"), Sent::Unread(_)));
}

#[test]
fn the_watcher_reads_a_terminal_as_it_reads_a_pane() {
    use crate::rewake::terminal_agent;
    use inbox_core::rewake::Status;
    let mut fake = FakeHc::recorded(GET_17);
    fake.terminal("t2")["status"] = json!({"state": "done"});
    let terminals = Hosted {
        hc: fake,
        machine: Box20::new(Record::Names(SESSION.into())),
    };
    assert_eq!(
        terminal_agent(&terminals, "t2"),
        Some((Some(SESSION.to_owned()), Status::Done))
    );
    assert_eq!(
        terminal_agent(&terminals, "t9"),
        None,
        "gone ends the watch"
    );
    let unread = Hosted {
        hc: FakeHc::recorded(GET_17),
        machine: Box20::new(Record::Missing),
    };
    assert_eq!(
        terminal_agent(&unread, "t2"),
        Some((None, Status::Other)),
        "no proof of anything: the watch waits"
    );
}

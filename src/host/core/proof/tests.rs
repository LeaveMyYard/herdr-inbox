use herdr_agent_kit::agent_procs::fake::FakeProcesses;
use serde_json::json;

use super::*;
use crate::host::core::fake::{FakeHc, GET_17, GET_18, LS_17, LS_18};

const MCP: Pid = 30;

/// A shell in t2, `claude` (pid 20) in the foreground, this server under it.
fn table() -> FakeProcesses {
    let mut t = FakeProcesses::default();
    t.insert(19, 1, "zsh", "s19", &["zsh"]);
    t.insert(20, 19, "claude", "s20", &["claude"]);
    t.insert(MCP, 20, "herdr-inbox", "s30", &["herdr-inbox", "mcp"]);
    t
}

fn hc() -> FakeHc {
    FakeHc::recorded(GET_17, LS_17)
}

fn refused(hc: &FakeHc, t: &FakeProcesses, claim: &str) -> Refused {
    prove(hc, t, 20, claim).unwrap_err()
}

#[test]
fn the_terminals_foreground_claude_is_proved() {
    let proved = prove(&hc(), &table(), 20, "t2").unwrap();
    assert_eq!(
        proved,
        Proved {
            pid: 20,
            start: "s20".into()
        }
    );
}

#[test]
fn the_0_1_18_recording_is_proved_too() {
    let mut t = FakeProcesses::default();
    t.insert(4242, 1, "claude", "s", &["/bin/sh", "/fixture/bin/claude"]);
    let fake = FakeHc::recorded(GET_18, LS_18);
    assert_eq!(prove(&fake, &t, 4242, "t1").unwrap().pid, 4242);
}

#[test]
fn a_walk_through_a_launcher_finds_the_same_claude() {
    let mut t = table();
    t.insert(25, 20, "sh", "s25", &["sh", "scripts/launch", "mcp"]);
    assert_eq!(prove(&hc(), &t, 25, "t2").unwrap().pid, 20);
}

#[test]
fn a_spoofed_terminal_is_refused() {
    // HC_TERMINAL_ID=t5 under a Claude that is not t5's foreground
    let mut fake = hc();
    let mut other: Value = serde_json::from_str(GET_17).unwrap();
    other["address"] = json!("t5");
    other["foreground"]["pid"] = json!(77);
    fake.set(other);
    assert_eq!(refused(&fake, &table(), "t5"), Refused::NotForeground);
}

#[test]
fn a_terminal_hc_does_not_know_is_refused() {
    assert_eq!(
        refused(&hc(), &table(), "t99"),
        Refused::Hc("hc read failed")
    );
}

#[test]
fn hc_missing_is_refused() {
    assert_eq!(
        refused(&FakeHc::gone(), &table(), "t2"),
        Refused::Hc("hc read failed")
    );
}

#[test]
fn a_nested_claude_is_refused_whichever_of_the_two_hc_reports() {
    let mut t = table();
    t.insert(40, 20, "claude", "s40", &["claude", "-p", "x"]);
    assert_eq!(prove(&hc(), &t, 40, "t2").unwrap_err(), Refused::Nested);
    let mut inner = hc();
    inner.terminal("t2")["foreground"]["pid"] = json!(40);
    assert_eq!(prove(&inner, &t, 40, "t2").unwrap_err(), Refused::Nested);
}

#[test]
fn a_process_under_no_claude_is_refused() {
    let t = table();
    assert_eq!(prove(&hc(), &t, 19, "t2").unwrap_err(), Refused::NoClaude);
}

#[test]
fn nothing_is_asked_of_hc_for_a_bad_or_remote_address() {
    for (claim, why) in [
        ("vps/t7", Refused::Remote),
        ("t3; touch x", Refused::BadAddress),
        ("--json", Refused::BadAddress),
        ("../t3", Refused::Remote),
        ("", Refused::BadAddress),
    ] {
        let fake = hc();
        assert_eq!(refused(&fake, &table(), claim), why, "{claim}");
        assert!(fake.asked.lock().unwrap().is_empty(), "{claim}");
    }
}

#[test]
fn hc_answering_for_another_address_is_refused() {
    let mut fake = hc();
    fake.terminal("t2")["address"] = json!("t3");
    assert_eq!(refused(&fake, &table(), "t2"), Refused::NotThatTerminal);
}

#[test]
fn a_finished_terminal_is_refused() {
    let mut fake = hc();
    fake.terminal("t2")["state"] = json!("finished");
    assert_eq!(refused(&fake, &table(), "t2"), Refused::NotThatTerminal);
}

#[test]
fn an_unknown_foreground_shape_is_refused_not_guessed() {
    for (field, value) in [
        ("foreground", json!(null)),
        ("foreground", json!({"name": "claude", "argv": ["claude"]})),
        (
            "foreground",
            json!({"pid": "20", "name": "claude", "argv": []}),
        ),
        (
            "foreground",
            json!({"pid": 1, "name": "claude", "argv": []}),
        ),
        (
            "foreground",
            json!({"pid": 4_294_967_316_u64, "name": "claude", "argv": []}),
        ),
        ("foreground", json!({"pid": 20, "argv": ["claude"]})),
        ("foreground", json!({"pid": 20, "name": "claude"})),
        (
            "foreground",
            json!({"pid": 20, "name": "claude", "argv": [1]}),
        ),
    ] {
        let mut fake = hc();
        fake.terminal("t2")[field] = value.clone();
        assert!(
            matches!(refused(&fake, &table(), "t2"), Refused::Hc(_)),
            "{value}"
        );
    }
}

#[test]
fn the_same_pid_with_another_name_or_argv_is_another_process() {
    // a pid from another pid namespace or another core instance
    let mut fake = hc();
    fake.terminal("t2")["foreground"]["name"] = json!("node");
    assert_eq!(refused(&fake, &table(), "t2"), Refused::NotThatProcess);
    let mut fake = hc();
    fake.terminal("t2")["foreground"]["argv"] = json!(["claude", "--resume", "x"]);
    assert_eq!(refused(&fake, &table(), "t2"), Refused::NotThatProcess);
}

#[test]
fn argv_compares_by_words() {
    let mut t = FakeProcesses::default();
    t.insert(20, 1, "claude", "s20", &["claude", "--model", "x y"]);
    let mut fake = hc();
    fake.terminal("t2")["foreground"]["argv"] = json!(["claude", "--model", "x", "y", ""]);
    assert!(prove(&fake, &t, 20, "t2").is_ok());
}

#[test]
fn hc_ls_must_list_it_once_and_live() {
    for terminals in [
        json!([]),
        json!([{"address": "t3", "state": "live"}]),
        json!([{"address": "t2", "state": "finished"}]),
        json!([{"address": "t2", "state": "live"}, {"address": "t2", "state": "live"}]),
    ] {
        let mut fake = hc();
        fake.set_list(json!({ "terminals": terminals }));
        assert_eq!(refused(&fake, &table(), "t2"), Refused::NotListed);
    }
    let mut fake = hc();
    fake.set_list(json!({"rows": []}));
    assert_eq!(
        refused(&fake, &table(), "t2"),
        Refused::Hc("hc ls has no terminals")
    );
}

#[test]
fn a_claude_with_no_start_token_proves_nothing() {
    let mut t = table();
    t.forget_start(20);
    assert_eq!(refused(&hc(), &t, "t2"), Refused::NoClaude);
}

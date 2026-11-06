use herdr_agent_kit::agent_procs::fake::FakeProcesses;
use serde_json::json;

use super::*;
use crate::host::core::fake::{FakeHc, GET_17, GET_18};

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
    FakeHc::recorded(GET_17)
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
    let fake = FakeHc::recorded(GET_18);
    assert_eq!(prove(&fake, &t, 4242, "t1").unwrap().pid, 4242);
}

#[test]
fn only_the_claudes_own_child_is_its_agent() {
    // `scripts/launch` execs the binary, so nothing sits between a real server and its Claude
    let mut t = table();
    t.insert(25, 20, "sh", "s25", &["sh", "-c", "herdr-inbox mcp"]);
    assert_eq!(
        prove(&hc(), &t, 25, "t2").unwrap_err(),
        Refused::NotItsChild
    );
}

#[test]
fn a_harness_the_kit_does_not_know_is_not_proved_as_its_parent() {
    // node with two options before the entry script is no Claude to the kit
    let entry = "/fixture/@anthropic-ai/claude-code/cli.js";
    let mut t = table();
    t.insert(50, 20, "node", "s50", &["node", "--a", "--b", entry]);
    assert_eq!(
        prove(&hc(), &t, 50, "t2").unwrap_err(),
        Refused::NotItsChild
    );
    assert!(hc().asked.lock().unwrap().is_empty());
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
        Refused::Hc(crate::host::core::hc::NO_TERMINAL)
    );
}

#[test]
fn hc_missing_is_refused() {
    assert_eq!(
        refused(&FakeHc::gone(), &table(), "t2"),
        Refused::Hc(crate::host::core::hc::NO_TERMINAL)
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
fn a_claude_with_no_start_token_proves_nothing() {
    let mut t = table();
    t.forget_start(20);
    assert_eq!(refused(&hc(), &t, "t2"), Refused::NoClaude);
}

#[test]
fn what_may_pass_in_a_moment_and_what_stays() {
    for why in [
        Refused::Hc("hc read timed out"),
        Refused::NotForeground,
        Refused::Replaced,
        Refused::BadRecord,
    ] {
        assert!(why.temporary(), "{why}");
    }
    for why in [
        Refused::BadAddress,
        Refused::Remote,
        Refused::NoClaude,
        Refused::NotItsChild,
        Refused::Nested,
        Refused::NotThatTerminal,
        Refused::NotThatProcess,
        Refused::NoSession,
    ] {
        assert!(!why.temporary(), "{why}");
    }
}

#[test]
fn a_hook_is_proved_through_the_shells_above_it() {
    let mut t = table();
    t.insert(30, 20, "sh", "s30", &["sh", "-c", "hook"]);
    t.insert(31, 30, "sh", "s31", &["sh", "launch"]);
    assert_eq!(
        prove_hook(&hc(), &t, 31, "t2"),
        Ok(Proved {
            pid: 20,
            start: "s20".into()
        })
    );
}

#[test]
fn a_hook_is_held_to_the_same_foreground_as_a_server() {
    let mut t = table();
    t.insert(30, 20, "sh", "s30", &["sh"]);
    // an inherited claim in another Claude: not that terminal's foreground
    t.insert(40, 1, "claude", "s40", &["claude"]);
    t.insert(41, 40, "sh", "s41", &["sh"]);
    assert_eq!(prove_hook(&hc(), &t, 41, "t2"), Err(Refused::NotForeground));
    // a Claude under the terminal's Claude
    t.insert(50, 30, "claude", "s50", &["claude", "-p"]);
    t.insert(51, 50, "sh", "s51", &["sh"]);
    assert_eq!(prove_hook(&hc(), &t, 51, "t2"), Err(Refused::Nested));
    assert_eq!(prove_hook(&hc(), &t, 30, "vps/t2"), Err(Refused::Remote));
    assert_eq!(prove_hook(&hc(), &t, 30, "--all"), Err(Refused::BadAddress));
    let shell = FakeProcesses::default();
    assert_eq!(prove_hook(&hc(), &shell, 7, "t2"), Err(Refused::NoClaude));
}

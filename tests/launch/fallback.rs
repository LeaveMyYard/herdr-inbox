use herdr_inbox::contract::CONTRACT;
use herdr_inbox::hook::output::hook_output;
use herdr_inbox::hook::rules::{SESSION_RULES, SUBAGENT_RULES};

use crate::world::repo;

#[test]
fn the_static_fallbacks_are_what_the_hook_prints() {
    let read = |f: &str| std::fs::read_to_string(repo().join("hooks/fallback").join(f)).unwrap();
    assert_eq!(
        read("session-start.json"),
        format!("{}\n", hook_output("SessionStart", SESSION_RULES))
    );
    assert_eq!(
        read("subagent-start.json"),
        format!("{}\n", hook_output("SubagentStart", SUBAGENT_RULES))
    );
}

#[test]
fn the_launcher_carries_the_binarys_contract() {
    let launch = std::fs::read_to_string(repo().join("scripts/launch")).unwrap();
    assert!(
        launch.contains(&format!("\nCONTRACT={CONTRACT}\n")),
        "scripts/launch and src/contract.rs move together"
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_herdr-inbox"))
        .arg("contract")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{CONTRACT}\n")
    );
}

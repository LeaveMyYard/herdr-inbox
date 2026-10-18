use crate::world::{Root, stand_in, stdout};

#[test]
fn the_newest_binary_with_the_launchers_contract_runs_the_hook() {
    let r = Root::new();
    stand_in(
        &r.state_bin().join("herdr-inbox-0.10.9"),
        "state-0.10.9",
        "0.10.9",
        1,
        0,
    );
    stand_in(
        &r.data_bin().join("herdr-inbox-0.11.0"),
        "data-0.11.0",
        "0.11.0",
        1,
        0,
    );
    stand_in(
        &r.data_bin().join("herdr-inbox-0.12.0"),
        "data-0.12.0",
        "0.12.0",
        2,
        0,
    ); // another contract
    stand_in(
        &r.state_bin().join("herdr-inbox-0.9.99"),
        "state-0.9.99",
        "0.9.99",
        1,
        0,
    );
    let out = r.sh("launch", &["hook", "stop"], "{\"payload\":1}", &[]);
    assert_eq!(
        stdout(&out),
        "data-0.11.0 stop\n{\"payload\":1}",
        "newest with contract 1; stdin passed through"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn versions_compare_numerically_across_both_directories() {
    let r = Root::new();
    stand_in(
        &r.data_bin().join("herdr-inbox-0.9.0"),
        "data-0.9.0",
        "0.9.0",
        1,
        0,
    );
    stand_in(
        &r.state_bin().join("herdr-inbox-0.10.0"),
        "state-0.10.0",
        "0.10.0",
        1,
        0,
    );
    stand_in(
        &r.state_bin().join("herdr-inbox-0.10.0.tmp"),
        "junk",
        "0",
        1,
        0,
    );
    stand_in(
        &r.state_bin().join(".herdr-inbox-0.99.0.123"),
        "junk",
        "0",
        1,
        0,
    );
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "state-0.10.0 stop\n"
    );
}

#[test]
fn a_linked_checkouts_own_build_comes_first() {
    let r = Root::new();
    stand_in(
        &r.data_bin().join("herdr-inbox-9.9.9"),
        "data",
        "9.9.9",
        1,
        0,
    );
    stand_in(
        &r.root().join("target/release/herdr-inbox"),
        "dev",
        "0.1.0",
        1,
        0,
    );
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "dev stop\n"
    );
    stand_in(
        &r.root().join("target/release/herdr-inbox"),
        "dev",
        "0.1.0",
        7,
        0,
    );
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "data stop\n",
        "not when its contract differs"
    );
}

#[test]
fn the_wake_code_passes_through_and_nothing_else_is_printed() {
    let r = Root::new();
    stand_in(&r.data_bin().join("herdr-inbox-1.0.0"), "w", "1.0.0", 1, 3);
    let out = r.sh("launch", &["hook", "rewake-watch"], "", &[]);
    assert_eq!(
        (out.status.code(), stdout(&out).as_str(), out.stderr.len()),
        (Some(3), "w rewake-watch\n", 0)
    );
}

#[test]
fn without_a_binary_the_python_hooks_run_then_the_static_rules() {
    let r = Root::new();
    stand_in(
        &r.data_bin().join("herdr-inbox-1.0.0"),
        "old",
        "1.0.0",
        0,
        0,
    ); // wrong contract only
    r.with_python();
    let out = r.sh("launch", &["hook", "rewake-watch"], "{\"p\":2}", &[]);
    assert_eq!(
        (out.status.code(), stdout(&out).as_str()),
        (Some(3), "python rewake-watch\n{\"p\":2}")
    );
    let bare = Root::new();
    let rules =
        std::fs::read_to_string(crate::world::repo().join("hooks/fallback/session-start.json"))
            .unwrap();
    let pane = [("HERDR_PANE_ID", std::path::Path::new("w1:p1"))];
    assert_eq!(
        stdout(&bare.sh("launch", &["hook", "session-start"], "{}", &pane)),
        rules
    );
    // the hooks' first gate: outside a herdr pane nobody watches the inbox (Python prints nothing)
    for event in ["session-start", "subagent-start"] {
        let out = bare.sh("launch", &["hook", event], "{}", &[]);
        assert_eq!(
            (out.status.code(), stdout(&out).as_str()),
            (Some(0), ""),
            "{event} outside herdr"
        );
    }
    for event in ["stop", "post-tool-batch", "rewake-watch", "nonsense"] {
        let out = bare.sh("launch", &["hook", event], "{}", &[]);
        assert_eq!(
            (out.status.code(), stdout(&out).as_str()),
            (Some(0), ""),
            "{event}"
        );
    }
    let out = bare.sh("launch", &[], "", &[]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
}

#[test]
fn a_directory_with_a_backslash_still_runs_its_binary() {
    // dash's echo would turn `\b` into a backspace: the exec would miss and the hook be lost
    let r = Root::new();
    let data = r.dir.path().join("da\\bta");
    stand_in(
        &data.join("bin/herdr-inbox-1.0.0"),
        "slashed",
        "1.0.0",
        1,
        0,
    );
    let out = r.sh(
        "launch",
        &["hook", "stop"],
        "{}",
        &[("CLAUDE_PLUGIN_DATA", &data)],
    );
    assert_eq!(
        (out.status.code(), stdout(&out).as_str()),
        (Some(0), "slashed stop\n{}")
    );
}

#[test]
fn the_linked_builds_probe_never_reads_the_hooks_payload() {
    let r = Root::new();
    stand_in(
        &r.root().join("target/release/herdr-inbox"),
        "dev",
        "0.1.0",
        1,
        0,
    );
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "{\"p\":1}", &[])),
        "dev stop\n{\"p\":1}"
    );
}

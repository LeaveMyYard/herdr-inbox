//! `scripts/launch cli`, `mcp` and `which`: what the `engine` file and the
//! binaries on disk decide, and that Python answers whenever a binary cannot.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::world::{Root, repo, stand_in, stdout};

/// A stand-in `uv` that says what it was asked to run.
fn with_uv(r: &Root) -> String {
    let dir = r.dir.path().join("fakeuv");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("uv"), "#!/bin/sh\necho \"uv $*\"\n").unwrap();
    fs::set_permissions(dir.join("uv"), fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}:{}",
        dir.display(),
        r.dir.path().join("tools").display(),
        std::env::var("PATH").unwrap()
    )
}

fn launch(r: &Root, args: &[&str]) -> String {
    let path = with_uv(r);
    stdout(&r.sh("launch", args, "", &[("PATH", Path::new(&path))]))
}

/// What Python's `inbox <args>` looks like through the stand-in uv.
fn python(r: &Root, args: &str) -> String {
    format!(
        "uv run --quiet --frozen --no-dev --project {} inbox {args}\n",
        r.root().display()
    )
}

fn recorded(r: &Root) {
    stand_in(
        &r.state_bin().join("herdr-inbox-1.0.0"),
        "state",
        "1.0.0",
        2,
        0,
    );
}

/// The commands the launcher gives the binary, from its one `case` line.
fn rust_cli() -> Vec<String> {
    let launch = fs::read_to_string(repo().join("scripts/launch")).unwrap();
    let line = launch
        .lines()
        .find(|l| l.trim_start().starts_with("list | answer"))
        .expect("the RUST_CLI case");
    line.trim()
        .trim_end_matches(')')
        .split(" | ")
        .map(String::from)
        .collect()
}

#[test]
fn every_rust_cli_command_is_one_the_binary_has() {
    let cmds = rust_cli();
    assert!(cmds.len() >= 10, "{cmds:?}");
    for cmd in &cmds {
        assert!(
            cmd == "worker" || herdr_inbox::cli::handles(cmd),
            "`{cmd}` goes to a binary that has no such command"
        );
    }
}

#[test]
fn the_binary_runs_its_commands_with_their_arguments() {
    let r = Root::new();
    recorded(&r);
    assert_eq!(
        launch(&r, &["cli", "worker", "--ensure"]),
        "state worker --ensure\n"
    );
    assert_eq!(
        launch(&r, &["cli", "answer", "Q1", "a b"]),
        "state answer Q1 a b\n"
    );
    assert_eq!(launch(&r, &["mcp"]), "state mcp\n");
    assert_eq!(
        launch(&r, &["which"]),
        format!("{}\n", r.state_bin().join("herdr-inbox-1.0.0").display())
    );
}

#[test]
fn what_the_binary_lacks_stays_python() {
    let r = Root::new();
    recorded(&r);
    for cmd in ["redeliver", "clean-venvs", "verify", "remote", "tui"] {
        assert_eq!(
            launch(&r, &["cli", cmd, "x"]),
            python(&r, &format!("{cmd} x")),
            "`{cmd}` must not reach a binary without it"
        );
    }
    assert_eq!(
        launch(&r, &["cli"]),
        python(&r, "").trim_end().to_string() + "\n"
    );
}

#[test]
fn with_no_binary_python_runs_all_of_it() {
    let r = Root::new();
    assert_eq!(
        launch(&r, &["cli", "worker", "--ensure"]),
        python(&r, "worker --ensure")
    );
    assert_eq!(launch(&r, &["cli", "list"]), python(&r, "list"));
    assert_eq!(launch(&r, &["mcp"]), python(&r, "mcp"));
    assert_eq!(launch(&r, &["which"]), "");
}

#[test]
fn a_binary_that_does_not_fit_is_passed_over_for_python() {
    // a hooks-only build: contract 1, whose `mcp` would only print its version
    let r = Root::new();
    stand_in(&r.root().join("bin/herdr-inbox"), "old", "1.0.0", 1, 0);
    assert_eq!(launch(&r, &["mcp"]), python(&r, "mcp"));
    // a file that does not run at all
    let dead = r.state_bin().join("herdr-inbox-2.0.0");
    fs::write(&dead, "not a program").unwrap();
    fs::set_permissions(&dead, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(launch(&r, &["cli", "list"]), python(&r, "list"));
    fs::set_permissions(&dead, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(launch(&r, &["cli", "list"]), python(&r, "list"));
}

#[test]
fn the_engine_file_alone_turns_every_binary_off() {
    for off in ["python\n", "", "rust \n", "Rust\n", "# rust\n"] {
        let r = Root::new();
        recorded(&r);
        r.with_python();
        r.engine(off);
        assert_eq!(launch(&r, &["cli", "list"]), python(&r, "list"), "{off:?}");
        assert_eq!(launch(&r, &["mcp"]), python(&r, "mcp"), "{off:?}");
        assert_eq!(launch(&r, &["which"]), "", "{off:?}");
        assert_eq!(launch(&r, &["hook", "stop"]), "python stop\n", "{off:?}");
    }
    let r = Root::new();
    recorded(&r);
    fs::remove_file(r.root().join("engine")).unwrap();
    assert_eq!(launch(&r, &["cli", "list"]), python(&r, "list"), "no file");
    let r = Root::new();
    recorded(&r);
    r.engine("rust"); // no newline: still one word
    assert_eq!(launch(&r, &["cli", "list"]), "state list\n");
}

#[test]
fn python_fetches_nothing() {
    let r = Root::new();
    r.engine("python\n");
    let sha = crate::release::publish(&r, "1.0.0");
    crate::release::release_toml(&r, "1.0.0", &sha);
    assert_eq!(launch(&r, &["mcp"]), python(&r, "mcp"));
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        r.tool_log(),
        "",
        "the launcher went for a binary it may not run"
    );
}

#[test]
fn the_installs_own_binary_goes_before_a_recorded_newer_one() {
    // the popup's root and the worker's install come from where the binary sits
    let r = Root::new();
    recorded(&r);
    stand_in(&r.root().join("bin/herdr-inbox"), "own", "0.9.0", 2, 0);
    assert_eq!(launch(&r, &["cli", "open"]), "own open\n");
}

#[test]
fn exit_codes_pass_through() {
    let r = Root::new();
    stand_in(&r.root().join("bin/herdr-inbox"), "own", "1.0.0", 2, 3);
    let path = with_uv(&r);
    let out = r.sh(
        "launch",
        &["cli", "open"],
        "",
        &[("PATH", Path::new(&path))],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "`inbox open`: the popup is already open"
    );
}

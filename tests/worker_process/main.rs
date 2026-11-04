//! Real worker processes: Rust workers against each other and against the
//! Python worker (plan §5.4, "Worker handover"). This executable is its own
//! test runner (`harness = false`) and, run as `<exe> worker`, a Rust
//! worker: the library's lifecycle in [`herdr_inbox::worker::live::Live`]
//! with recording duties ([`child`]), since the binary gets its `worker`
//! subcommand only with the duties (P5-E2).
//!
//! Every process runs with an environment built here from nothing (no
//! `HERDR_*` of the session reaches one), against a fake herdr (a socket
//! for the Rust workers, a CLI for the Python ones) and a database in a
//! temporary directory. Tests signal only the children they spawned and
//! still hold; a worker one of those started ends on its own, by the rule
//! under test: it exits when its herdr server is gone.

mod child;
mod exclusion;
mod exits;
mod handover;
mod rig;
mod roots;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

type Case = (&'static str, fn());

const CASES: &[Case] = &[
    (
        "handover::python_hands_over_to_rust_on_an_update",
        handover::python_hands_over_to_rust_on_an_update,
    ),
    (
        "handover::rust_hands_over_to_python_on_an_update",
        handover::rust_hands_over_to_python_on_an_update,
    ),
    (
        "handover::an_r1_python_holder_hands_over_to_an_r2_rust_standby",
        handover::an_r1_python_holder_hands_over_to_an_r2_rust_standby,
    ),
    (
        "handover::a_marker_gets_an_r1_rust_holder_an_r2_python_successor",
        handover::a_marker_gets_an_r1_rust_holder_an_r2_python_successor,
    ),
    (
        "handover::a_marker_reaches_the_listed_install_from_a_holder_herdr_does_not_list",
        handover::a_marker_reaches_the_listed_install_from_a_holder_herdr_does_not_list,
    ),
    (
        "exclusion::the_newer_worker_wins_the_lock_race_whatever_the_start_order",
        exclusion::the_newer_worker_wins_the_lock_race_whatever_the_start_order,
    ),
    (
        "exclusion::two_workers_are_never_both_live",
        exclusion::two_workers_are_never_both_live,
    ),
    (
        "exits::the_rust_worker_exits_once_its_server_is_gone",
        exits::the_rust_worker_exits_once_its_server_is_gone,
    ),
    (
        "exits::the_rust_worker_exits_when_idle",
        exits::the_rust_worker_exits_when_idle,
    ),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("worker") {
        return child::main();
    }
    if args.iter().any(|a| a == "--list") {
        for (name, _) in CASES {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--ignored") {
        return ExitCode::SUCCESS; // none is ignored
    }
    let exact = args.iter().any(|a| a == "--exact");
    let filters: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(String::as_str)
        .collect();
    let chosen = |name: &str| {
        filters.is_empty()
            || filters
                .iter()
                .any(|f| if exact { name == *f } else { name.contains(f) })
    };
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, case) in CASES.iter().filter(|(n, _)| chosen(n)) {
        ran += 1;
        match catch_unwind(AssertUnwindSafe(case)) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    let verdict = if failed.is_empty() { "ok" } else { "FAILED" };
    let passed = ran - failed.len();
    println!(
        "\ntest result: {verdict}. {passed} passed; {} failed",
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

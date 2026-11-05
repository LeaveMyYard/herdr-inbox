//! The fake herdr CLI, where the tests lean on the order of its steps.

use std::fs;
use std::process::{Command, Stdio};

use crate::rig::{Rig, wait_for};

/// A Python worker decides once, from its first listing, whether it is
/// herdr's install, so a test may change the listing only after that answer
/// is settled. The listed root is a FIFO here: the CLI stops at reading it,
/// after the call and before the answer, and that is when a slow runner
/// would let the test's update through.
pub fn the_listing_counts_once_it_is_answered() {
    let rig = Rig::new();
    let listed = rig.listed_file();
    assert!(
        Command::new("mkfifo")
            .arg(listed)
            .status()
            .unwrap()
            .success()
    );
    let cli = rig
        .herdr_cli(&["plugin", "list", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(
        wait_for(|| rig.herdr_called(" plugin list --json")),
        "{}",
        rig.logs()
    );
    let early = rig.python_asked_for_its_root();
    fs::write(listed, "/the/root").unwrap(); // before any assert: the CLI must not be left waiting
    let out = cli.wait_with_output().unwrap();
    assert!(!early, "ready before the listing was read\n{}", rig.logs());
    assert!(String::from_utf8_lossy(&out.stdout).contains("/the/root"));
    assert!(rig.python_asked_for_its_root(), "{}", rig.logs());
}

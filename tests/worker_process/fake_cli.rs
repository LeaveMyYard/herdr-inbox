//! The fake herdr CLI, where the tests lean on the order of its steps.

use std::fs;
use std::path::Path;
use std::process::Stdio;

use crate::rig::{LIMIT, Rig, wait_for};

/// A `cat` that waits for the file `go` before it reads, and gives up waiting
/// by itself after [`LIMIT`], so whatever this test does the fake ends.
fn stalled_cat(go: &Path) -> String {
    let polls = LIMIT.as_secs() * 20;
    format!(
        "#!/bin/sh\ni=0\nwhile [ ! -e '{}' ] && [ \"$i\" -lt {polls} ]; do sleep 0.05; i=$((i + 1)); done\nexec /bin/cat \"$@\"\n",
        go.display()
    )
}

/// A Python worker decides once, from its first listing, whether it is
/// herdr's install, so a test may change the listing only after that answer
/// is settled. The CLI stalls here at reading the listed root, after the call
/// and before the answer: where a slow runner lets the test's update through.
pub fn the_listing_counts_once_it_is_answered() {
    let rig = Rig::new();
    rig.list(Path::new("/the/root"));
    let go = rig.dir.path().join("go");
    rig.on_path("cat", &stalled_cat(&go));
    let cli = rig
        .herdr_cli(&["plugin", "list", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let called = wait_for(|| rig.herdr_called(" plugin list --json"));
    let early = rig.python_has_its_listing();
    fs::write(&go, "").unwrap();
    let out = cli.wait_with_output().unwrap();
    assert!(called, "{}", rig.logs());
    assert!(!early, "ready before the listing was read\n{}", rig.logs());
    assert!(String::from_utf8_lossy(&out.stdout).contains("/the/root"));
    assert!(rig.python_has_its_listing(), "{}", rig.logs());
}

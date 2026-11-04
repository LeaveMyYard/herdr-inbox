//! A Python worker and a Rust worker hand the lock to each other (plan
//! §5.4): on an update (the version under herdr's listed root changed), and
//! to a newer revision (herdr-inbox #50: a standby, and the revision-wanted
//! marker the Rust hooks write).

use herdr_inbox::store::revision_wanted::{unix_now, want};
use herdr_inbox::worker::handover::standby_path;
use herdr_inbox::worker::live::db_stamp;

use crate::rig::{Rig, is_python_worker, wait_for};
use crate::roots::{python_root, with_hooks_only_binary, with_rust_worker};

/// Python `start_successor` finds the Rust worker under the new install
/// and the Rust worker takes over the lock; the Python worker exits.
pub fn python_hands_over_to_rust_on_an_update() {
    let mut rig = Rig::new();
    let old = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&old);
    let python = rig.start_python("python", &old);
    let ready = || rig.held_by(1, python) && rig.python_asked_for_its_root();
    assert!(wait_for(ready), "{}", rig.logs());
    let new = python_root(rig.dir.path(), "b", "0.10.7", 1);
    with_rust_worker(&new);
    rig.list(&new); // `herdr plugin update` lands in a new directory
    let status = rig.must_exit(python, "the Python worker exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    let ticked = || rig.ticked();
    assert!(
        wait_for(|| !ticked().is_empty()),
        "the Rust worker serves\n{}",
        rig.logs()
    );
    let rust = rig.ticked()[0];
    assert!(
        rig.held_by(1, rust),
        "{:?}: the Rust worker holds the lock",
        rig.holder()
    );
    assert_eq!(rig.ticked(), [rust], "one Rust worker, no handover back");
    rig.finish(&[python, rust]);
}

/// The reverse: the Rust worker sees the update, the new install has no
/// Rust worker, so `uv run … inbox worker` starts its Python worker, which
/// takes the lock; the Rust worker exits.
pub fn rust_hands_over_to_python_on_an_update() {
    let mut rig = Rig::new();
    let old = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&old);
    let rust = rig.start_rust("rust", &old, 1, &[]);
    assert!(
        wait_for(|| rig.held_by(1, rust) && rig.ticked() == [rust]),
        "{}",
        rig.logs()
    );
    let new = python_root(rig.dir.path(), "b", "0.10.7", 1);
    rig.list(&new);
    let status = rig.must_exit(rust, "the Rust worker exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    assert!(
        wait_for(|| rig.holder().is_some_and(|(_, pid)| pid != rust)
            && rig.held_by(1, rig.holder().unwrap().1)),
        "{:?}\n{}",
        rig.holder(),
        rig.logs()
    );
    let python = rig.holder().unwrap().1;
    assert!(
        is_python_worker(python),
        "pid {python} is the new install's Python worker"
    );
    rig.finish(&[rust, python]);
}

/// #50: an r1 Python worker holds the lock; an r2 Rust worker stands by
/// (its standby record, byte for byte Python's) and the Python worker hands
/// the lock over to it. r1 workers started after the takeover (r1 MCP
/// servers, popup and CLI keep calling `ensure()`) never hold it again.
pub fn an_r1_python_holder_hands_over_to_an_r2_rust_standby() {
    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&root);
    let python = rig.start_python("python", &root);
    assert!(wait_for(|| rig.held_by(1, python)), "{}", rig.logs());
    let rust = rig.start_rust("rust", &root, 2, &[]);
    let status = rig.must_exit(python, "the r1 holder hands over and exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    assert!(
        wait_for(|| rig.held_by(2, rust)),
        "{:?}\n{}",
        rig.holder(),
        rig.logs()
    );
    assert!(wait_for(|| rig.ticked() == [rust]));
    assert!(
        !standby_path(&rig.lock()).exists(),
        "the standby's record went with it"
    );
    let mut late = Vec::new();
    for n in 0..3 {
        let r1 = rig.start_python(&format!("late-r1-{n}"), &root);
        let status = rig.must_exit(r1, "an r1 worker beside an r2 holder exits");
        assert!(
            status.success() && rig.held_by(2, rust),
            "{status:?}\n{}",
            rig.logs()
        );
        late.push(r1);
    }
    late.extend([python, rust]);
    rig.finish(&late);
}

/// Python test_a_rust_hooks_marker_alone_gets_the_r2_worker_of_a_release_install,
/// with a Rust holder: herdr lists an r2 install whose `bin/herdr-inbox` has
/// no worker; the marker makes the r1 Rust holder start that install's
/// Python worker, which stands by, takes over and migrates to r2.
pub fn a_marker_gets_an_r1_rust_holder_an_r2_python_successor() {
    marker_case(true);
}

/// Python test_a_marker_reaches_the_listed_install_from_a_holder_herdr_does_not_list:
/// the holder runs from a copy herdr never lists (a Claude-cache copy); the
/// marker still gets herdr's listed r2 install started.
pub fn a_marker_reaches_the_listed_install_from_a_holder_herdr_does_not_list() {
    marker_case(false);
}

fn marker_case(holder_listed_first: bool) {
    let mut rig = Rig::new();
    let r1 = python_root(rig.dir.path(), "r1", "0.10.6", 1);
    let r2 = python_root(rig.dir.path(), "r2", "0.10.6", 2);
    with_hooks_only_binary(&r2);
    rig.list(if holder_listed_first { &r1 } else { &r2 });
    let rust = rig.start_rust("rust", &r1, 1, &[]);
    assert!(
        wait_for(|| rig.held_by(1, rust) && rig.ticked() == [rust]),
        "{}",
        rig.logs()
    );
    rig.list(&r2);
    want(&rig.db, 2, unix_now()); // what a not-ready Rust hook writes
    let status = rig.must_exit(rust, "the r1 holder hands over and exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    let python = || {
        rig.holder()
            .filter(|(rev, _)| *rev == 2)
            .map(|(_, pid)| pid)
    };
    let migrated = || python().is_some() && db_stamp(&rig.db) == Some(2);
    assert!(wait_for(migrated), "{:?}\n{}", rig.holder(), rig.logs());
    let python = python().unwrap();
    assert!(rig.held_by(2, python) && is_python_worker(python));
    rig.finish(&[rust, python]);
}

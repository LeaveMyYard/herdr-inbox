//! A Python worker and a Rust worker hand the lock to each other (plan
//! §5.4): on an update (the version under herdr's listed root changed), and
//! to a newer revision (herdr-inbox #50: a standby, and the revision-wanted
//! marker the Rust hooks write).

use herdr_inbox::store::revision::MIGRATION_REVISION;
use herdr_inbox::store::revision_wanted::{unix_now, want};
use herdr_inbox::worker::handover::standby_path;
use herdr_inbox::worker::live::db_stamp;
use herdr_inbox::worker::lock::is_running;

use crate::rig::{Rig, is_python_worker, wait_for};
use crate::roots::{python_root, with_hooks_only_binary, with_rust_worker};

/// Python `start_successor` finds the Rust worker under the new install
/// and the Rust worker takes over the lock, from that install; the Python
/// worker exits.
pub fn python_hands_over_to_rust_on_an_update() {
    let mut rig = Rig::new();
    let old = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&old);
    let python = rig.start_python("python", &old);
    let ready = || rig.held_by(1, python) && rig.python_has_its_listing();
    assert!(wait_for(ready), "{}", rig.logs());
    let new = python_root(rig.dir.path(), "b", "0.10.7", 1);
    with_rust_worker(&new);
    rig.list(&new); // `herdr plugin update` lands in a new directory
    let status = rig.must_exit(python, "the Python worker exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    let ticked = || rig.ticked();
    let serves = wait_for(|| !ticked().is_empty());
    assert!(serves, "the Rust worker serves\n{}", rig.logs());
    let rust = rig.ticked()[0];
    assert!(
        rig.held_by(MIGRATION_REVISION, rust),
        "{:?}: the Rust worker holds it",
        rig.holder()
    );
    assert_eq!(
        rig.tick_root(rust),
        Some(new.clone()),
        "started from the new install"
    );
    assert_eq!(rig.ticked(), [rust], "one Rust worker, no handover back");
    rig.finish(&[python, rust]);
}

/// The reverse: the Rust worker sees the update, the new install has no
/// Rust worker, so `uv run … --project <new install> inbox worker` starts
/// that install's Python worker, which takes the lock; the Rust worker exits.
pub fn rust_hands_over_to_python_on_an_update() {
    let mut rig = Rig::new();
    let old = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&old);
    let rust = rig.start_rust("rust", &old, 1, &[]);
    let serving = || rig.held_by(1, rust) && rig.ticked() == [rust];
    assert!(wait_for(serving), "{}", rig.logs());
    let new = python_root(rig.dir.path(), "b", "0.10.7", 1);
    rig.list(&new);
    let status = rig.must_exit(rust, "the Rust worker exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    let other = || {
        rig.holder()
            .filter(|(_, pid)| *pid != rust)
            .map(|(_, pid)| pid)
    };
    let taken = || other().is_some_and(|pid| rig.held_by(1, pid));
    assert!(wait_for(taken), "{:?}\n{}", rig.holder(), rig.logs());
    let python = other().unwrap();
    assert_eq!(
        rig.uv_roots(),
        std::slice::from_ref(&new),
        "uv ran the new install's worker"
    );
    let command = python_command(python);
    let src = format!("{} worker", new.join("src").display());
    assert!(
        is_python_worker(python) && command.ends_with(&src),
        "{command}"
    );
    rig.finish(&[rust, python]);
}

/// A Rust worker hands over to the new install's Rust worker, which runs
/// from the new install.
pub fn rust_hands_over_to_rust_on_an_update() {
    let mut rig = Rig::new();
    let old = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&old);
    let first = rig.start_rust("rust", &old, 1, &[]);
    assert!(wait_for(|| rig.ticked() == [first]), "{}", rig.logs());
    let new = python_root(rig.dir.path(), "b", "0.10.7", 1);
    with_rust_worker(&new);
    rig.list(&new);
    let status = rig.must_exit(first, "the old Rust worker exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    assert!(
        wait_for(|| rig.ticked().len() == 2),
        "{:?}\n{}",
        rig.ticked(),
        rig.logs()
    );
    let second = rig.ticked()[1];
    assert!(rig.held_by(1, second));
    assert_eq!(
        rig.tick_root(second),
        Some(new),
        "started from the new install"
    );
    assert!(rig.uv_roots().is_empty(), "no Python worker");
    rig.finish(&[first, second]);
}

/// A linked checkout pulled but not rebuilt: the version on disk changes in
/// place under the same `target/release/herdr-inbox`. The worker hands over
/// once to that binary, which reads the new version from disk and stays (a
/// worker comparing the compiled version would start itself every tick).
pub fn a_checkout_pulled_but_not_rebuilt_hands_over_once() {
    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "linked", "0.10.6", 1);
    with_rust_worker(&root);
    rig.list(&root);
    let first = rig.start_rust("rust", &root, 1, &[]);
    assert!(wait_for(|| rig.ticked() == [first]), "{}", rig.logs());
    let pyproject = root.join("pyproject.toml");
    let text = std::fs::read_to_string(&pyproject).unwrap();
    let pulled = text.replacen("version = \"0.10.6\"", "version = \"0.10.7\"", 1);
    std::fs::write(&pyproject, pulled).unwrap(); // `git pull`, no `cargo build`
    let status = rig.must_exit(first, "the worker hands over");
    assert!(status.success(), "{status:?}");
    assert!(
        wait_for(|| rig.ticked().len() == 2),
        "{:?}\n{}",
        rig.ticked(),
        rig.logs()
    );
    std::thread::sleep(std::time::Duration::from_secs(3)); // 15 ticks: no handover loop
    let ticked = rig.ticked();
    assert_eq!(ticked.len(), 2, "{ticked:?}: one handover, then it stays");
    assert!(rig.held_by(1, ticked[1]));
    rig.finish(&ticked);
}

/// The command line of `pid`.
fn python_command(pid: u32) -> String {
    let out = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// #50, Python test_r2_hooks_alone_get_an_r2_worker_while_r1_ensure_callers_fire
/// across the languages: an r1 Python worker holds the lock; an r2 Rust
/// worker stands by (its standby record, byte for byte Python's) and the
/// Python worker hands the lock over to it. r1 workers keep being started
/// all through the handover and after it (r1 MCP servers, popup and CLI
/// calling `ensure()`): once r2 holds, no r1 worker ever holds again.
pub fn an_r1_python_holder_hands_over_to_an_r2_rust_standby() {
    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&root);
    let python = rig.start_python("python", &root);
    assert!(wait_for(|| rig.held_by(1, python)), "{}", rig.logs());
    let rust = rig.start_rust("rust", &root, 2, &[]);
    let (mut callers, mut seen, mut after) = (Vec::new(), Vec::new(), 0);
    let deadline = std::time::Instant::now() + crate::rig::LIMIT;
    while after < 10 && std::time::Instant::now() < deadline {
        let n = callers.len();
        callers.push(rig.start_python(&format!("r1-caller-{n}"), &root));
        for _ in 0..5 {
            seen.push(rig.holder().filter(|_| is_running(&rig.lock())));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if rig.held_by(2, rust) {
            after += 1;
        }
    }
    assert_eq!(after, 10, "r2 took over\n{}", rig.logs());
    let status = rig.must_exit(python, "the r1 holder hands over and exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    let first_r2 = seen
        .iter()
        .position(|h| h.is_some_and(|(rev, _)| rev == 2))
        .unwrap();
    let r1_after = seen[first_r2..]
        .iter()
        .flatten()
        .filter(|(rev, _)| *rev == 1)
        .count();
    assert_eq!(r1_after, 0, "an r1 worker held after r2: {seen:?}");
    assert!(rig.held_by(2, rust) && rig.ticked() == [rust]);
    assert!(
        !standby_path(&rig.lock()).exists(),
        "the standby's record went with it"
    );
    for caller in &callers {
        let status = rig.must_exit(*caller, "an r1 caller's worker exits");
        assert!(status.success(), "{status:?}");
    }
    callers.extend([python, rust]);
    rig.finish(&callers);
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

/// Python `_spawn_newer` → `_rust_stands_by`: an r1 Python holder sees the
/// marker, starts the listed install's Rust worker (r2 here), which stands
/// by, and hands the lock over to it.
pub fn a_marker_gets_an_r1_python_holder_an_r2_rust_standby() {
    let mut rig = Rig::new();
    let r1 = python_root(rig.dir.path(), "r1", "0.10.6", 1);
    let r2 = python_root(rig.dir.path(), "r2", "0.10.6", 1);
    with_rust_worker(&r2);
    rig.list(&r1);
    // the Rust worker the Python holder starts inherits its environment
    let python = rig.start_python_with("python", &r1, &[("E1_REVISION", "2")]);
    let ready = || rig.held_by(1, python) && rig.python_has_its_listing();
    assert!(wait_for(ready), "{}", rig.logs());
    rig.list(&r2);
    want(&rig.db, 2, unix_now());
    let status = rig.must_exit(python, "the r1 holder hands over and exits");
    assert!(status.success(), "{status:?}\n{}", rig.logs());
    assert!(wait_for(|| rig.ticked().len() == 1), "{}", rig.logs());
    let rust = rig.ticked()[0];
    assert!(rig.held_by(2, rust), "{:?}", rig.holder());
    assert_eq!(rig.tick_root(rust), Some(r2), "the listed install's worker");
    assert!(rig.uv_roots().is_empty());
    rig.finish(&[python, rust]);
}

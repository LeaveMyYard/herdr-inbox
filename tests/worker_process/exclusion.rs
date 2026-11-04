//! The lock keeps two workers from both being live, a Python and a Rust one
//! alike, and the newer revision always ends up holding it.

use std::time::Duration;

use herdr_inbox::worker::handover::standby_path;

use crate::rig::{Rig, wait_for};
use crate::roots::python_root;

#[derive(Clone, Copy, Debug)]
enum Side {
    Python,
    Rust,
}

fn start(rig: &mut Rig, side: Side, revision: i64) -> u32 {
    let name = format!("{side:?}-r{revision}");
    let root = python_root(rig.dir.path(), &name, "0.10.6", revision);
    if revision == 1 {
        rig.list(&root);
    }
    match side {
        Side::Python => rig.start_python(&name, &root),
        Side::Rust => rig.start_rust(&name, &root, revision, &[]),
    }
}

/// Python test_the_newer_worker_wins_the_lock_race_whatever_the_start_order,
/// across the languages: an r1 and an r2 worker started together, each
/// order, each side newer. The r2 one holds the lock; the r1 one exits.
pub fn the_newer_worker_wins_the_lock_race_whatever_the_start_order() {
    let rounds = [
        (Side::Rust, 1, Side::Python, 2),
        (Side::Python, 2, Side::Rust, 1),
        (Side::Python, 1, Side::Rust, 2),
        (Side::Rust, 2, Side::Python, 1),
    ];
    for (round, (first, a_rev, second, b_rev)) in rounds.into_iter().enumerate() {
        let mut rig = Rig::new();
        let a = start(&mut rig, first, a_rev);
        let b = start(&mut rig, second, b_rev);
        let (older, newer) = if a_rev == 1 { (a, b) } else { (b, a) };
        let status = rig.wait_exit(older);
        assert!(
            status.is_some_and(|s| s.success()),
            "round {round}: {status:?}\n{}",
            rig.logs()
        );
        assert!(
            wait_for(|| rig.held_by(2, newer)),
            "round {round}: {:?}\n{}",
            rig.holder(),
            rig.logs()
        );
        let seq = rig.tick_sequence();
        let first_newer = seq.iter().position(|p| *p == newer).unwrap_or(seq.len());
        assert!(
            !seq[first_newer..].contains(&older),
            "round {round}: the older one served after the newer"
        );
        rig.finish(&[a, b]);
    }
}

/// Two Rust workers of one revision started together: one holds the lock,
/// the other exits at once, without ever ticking or standing by. A Rust
/// worker started beside a Python worker of its revision does the same.
pub fn two_workers_are_never_both_live() {
    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&root);
    let (a, b) = (
        rig.start_rust("a", &root, 1, &[]),
        rig.start_rust("b", &root, 1, &[]),
    );
    let first_out = loop {
        if let Some(status) = rig.exited(a) {
            break (a, b, status);
        }
        if let Some(status) = rig.exited(b) {
            break (b, a, status);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (loser, winner, status) = first_out;
    assert!(status.success(), "{status:?}");
    assert!(wait_for(|| rig.ticked() == [winner]), "{:?}", rig.ticked());
    std::thread::sleep(Duration::from_secs(1)); // a few ticks more: still only the winner
    assert!(rig.held_by(1, winner) && rig.ticked() == [winner] && rig.exited(winner).is_none());
    assert!(
        !standby_path(&rig.lock()).exists(),
        "the loser never stood by"
    );
    rig.finish(&[winner, loser]);

    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&root);
    let python = rig.start_python("python", &root);
    assert!(wait_for(|| rig.held_by(1, python)), "{}", rig.logs());
    let rust = rig.start_rust("rust", &root, 1, &[]);
    let status = rig.must_exit(rust, "the Rust worker exits");
    assert!(status.success() && rig.ticked().is_empty(), "{status:?}");
    assert!(
        rig.held_by(1, python) && rig.exited(python).is_none(),
        "the Python worker serves on"
    );
    rig.finish(&[python, rust]);
}

//! A Rust worker ends by itself: when its herdr server is gone (plan §4 item
//! 5: it is detached, so nothing else stops it), and when idle.

use std::time::{Duration, Instant};

use herdr_inbox::worker::SERVER_GONE_EXIT;
use herdr_inbox::worker::handover::holder_path;
use herdr_inbox::worker::lock::is_running;

use crate::rig::{Rig, wait_for};
use crate::roots::python_root;

/// Python test_the_worker_exits_once_the_server_goes_away, as a process:
/// it serves, the socket goes, it exits 0 after the grace (not before),
/// the lock free and its record gone.
pub fn the_rust_worker_exits_once_its_server_is_gone() {
    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&root);
    let rust = rig.start_rust("rust", &root, 1, &[]);
    assert!(
        wait_for(|| rig.held_by(1, rust) && rig.ticked() == [rust]),
        "{}",
        rig.logs()
    );
    let gone = Instant::now();
    rig.server_goes_away();
    std::thread::sleep(SERVER_GONE_EXIT - Duration::from_secs(2));
    assert!(
        rig.exited(rust).is_none(),
        "a moment without the server is no exit"
    );
    let status = rig.must_exit(rust, "it exits");
    let after = gone.elapsed();
    assert!(status.success(), "{status:?}");
    assert!(
        after >= SERVER_GONE_EXIT && after < SERVER_GONE_EXIT * 2,
        "{after:?}"
    );
    assert!(!is_running(&rig.lock()) && !holder_path(&rig.lock()).exists());
    rig.finish(&[rust]);
}

/// Python test_worker_process_runs_and_exits_when_idle: nothing pending and
/// nothing kept alive for the idle time: it exits 0 and lets go.
pub fn the_rust_worker_exits_when_idle() {
    let mut rig = Rig::new();
    let root = python_root(rig.dir.path(), "a", "0.10.6", 1);
    rig.list(&root);
    let idle = [("E1_IDLE", "1"), ("HERDR_INBOX_WORKER_IDLE_EXIT", "1")];
    let rust = rig.start_rust("rust", &root, 1, &idle);
    assert!(wait_for(|| rig.ticked() == [rust]), "{}", rig.logs());
    let status = rig.must_exit(rust, "it idles out");
    assert!(status.success(), "{status:?}");
    assert!(
        rig.tick_sequence().len() >= 3,
        "it ticked for the idle second first"
    );
    assert!(!is_running(&rig.lock()));
    rig.finish(&[rust]);
}

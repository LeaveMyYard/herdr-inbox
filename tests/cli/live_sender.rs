//! The detached sender behind the real binary (Python
//! `test_the_detached_sender_end_to_end`), no mocks: `apply` records the
//! answer and returns; its child `herdr-inbox deliver` delivers and verifies,
//! once.

use std::io::Write;
use std::process::Stdio;
use std::time::{Duration, Instant};

use herdr_inbox::remote_api::keys::{db_id, qkey};
use herdr_inbox::store::Store;

use super::{Rig, json, open, text};

/// Far past any honest delivery; only a broken one waits this long.
const LIMIT: Duration = Duration::from_secs(30);

/// `herdr-inbox apply` with an answer for Q1 on stdin: its reply.
fn apply_answer(rig: &Rig, rid: &str) -> String {
    let store = Store::open(&rig.db, Duration::from_secs(5)).unwrap();
    let dbid = db_id(store.db()).unwrap();
    let key = qkey(&dbid, &store.get(1).unwrap().unwrap());
    let request = json!({"request_id": rid, "db_id": dbid, "id": 1, "qkey": key,
                         "action": "answer", "detail": {"text": "yes"}});
    let mut child = rig
        .binary(&["apply"], &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(request.to_string().as_bytes()).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    text(&out.stdout)
}

fn verified(rig: &Rig) -> bool {
    open(&rig.db)
        .query_row(
            "SELECT verified_at FROM question_ext WHERE id = 1",
            [],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .is_some()
}

fn wait_verified(rig: &Rig) -> bool {
    let start = Instant::now();
    while !verified(rig) && start.elapsed() < LIMIT {
        std::thread::sleep(Duration::from_millis(50));
    }
    verified(rig)
}

#[test]
fn the_detached_sender_delivers_and_verifies_once() {
    let rig = Rig::new();
    let reply = apply_answer(&rig, "r1");
    assert!(reply.contains("applied"), "{reply}");
    assert!(
        wait_verified(&rig),
        "the sender's log: {}",
        std::fs::read_to_string(rig.db.with_file_name("worker.log")).unwrap_or_default()
    );
    assert_eq!(rig.stub.params_of("agent.prompt").len(), 1);
    // the same request again: recorded once, so no second sender types it
    apply_answer(&rig, "r1");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rig.stub.params_of("agent.prompt").len(), 1);
}

#[test]
fn a_sender_whose_log_cannot_open_still_delivers() {
    let rig = Rig::new();
    std::fs::create_dir(rig.db.with_file_name("worker.log")).unwrap();
    apply_answer(&rig, "r2");
    assert!(wait_verified(&rig), "delivered without its log");
    assert_eq!(rig.stub.params_of("agent.prompt").len(), 1);
}

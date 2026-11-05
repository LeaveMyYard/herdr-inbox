//! A database error part way through a pass (#132): what the pass committed
//! before it keeps its effects. Python notifies as it goes, so an earlier
//! row's notification survives a later row's error; the store hands effects
//! back instead, and must not drop them with the error, because the committed
//! row is never picked up again. The faults are injected with triggers.
//! Synthetic data.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_inbox::clock::{iso_seconds, now_minus};
use herdr_inbox::pyjson::dumps;
use herdr_inbox::remote::command::{FixedCommand, PROBE, SafeTarget};
use herdr_inbox::store::Store;
use herdr_inbox::store::alerts::{BlockedScan, Effect, SeenAgent};
use herdr_inbox::store::mirror::{CallResult, Notice};
use rusqlite::params;
use serde_json::json;

use crate::common::migrated;

const EPOCH_2026: u64 = 1_790_000_000;
const INJECTED: &str = "injected fault";

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(EPOCH_2026 + secs)
}

fn store(dir: &tempfile::TempDir) -> Store {
    Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap()
}

/// Fails `event` on `table` for the rows `when` selects.
fn fault(store: &Store, event: &str, table: &str, when: &str) {
    store
        .db()
        .execute_batch(&format!(
            "CREATE TRIGGER fault BEFORE {event} ON {table} WHEN {when} \
             BEGIN SELECT RAISE(ABORT, '{INJECTED}'); END"
        ))
        .unwrap();
}

fn clear_fault(store: &Store) {
    store.db().execute_batch("DROP TRIGGER fault").unwrap();
}

fn titles(notices: &[Notice]) -> Vec<&str> {
    notices.iter().map(|n| n.title.as_str()).collect()
}

fn injected(error: Option<rusqlite::Error>) {
    let error = error.expect("the fault stops the pass").to_string();
    assert!(error.contains(INJECTED), "{error}");
}

fn blocked_agent(pane: &str) -> SeenAgent {
    SeenAgent {
        pane_id: pane.into(),
        status: Some("blocked".into()),
        tab_id: None,
        session: None,
        agent: Some("Agent Name 1".into()),
    }
}

/// `pane` has been at its prompt long enough to be alerted.
fn long_blocked(store: &Store, pane: &str) {
    store
        .db()
        .execute(
            "INSERT INTO blocked_panes (herdr_server, pane_id, since) VALUES ('/srv/a', ?, ?)",
            params![pane, now_minus(Duration::from_secs(60))],
        )
        .unwrap();
}

#[test]
fn an_alert_committed_before_a_later_panes_error_is_still_notified_and_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let agents = [blocked_agent("w1:p1"), blocked_agent("w1:p2")];
    long_blocked(&store, "w1:p1");
    long_blocked(&store, "w1:p2");
    fault(&store, "INSERT", "alerts", "NEW.pane_id = 'w1:p2'");
    let mut scan = BlockedScan::default();

    let got = scan.scan(&store, "/srv/a", Some(&agents), 0.0, |_| None);

    let (n, effects) = got.value;
    assert_eq!(n, 0, "a failed scan watches nobody");
    assert_eq!(
        effects,
        [
            Effect::Notify {
                title: "A1 · w1:p1".into(),
                body: "waiting for you at a prompt".into()
            },
            Effect::Markers(Some("w1:p1".into()))
        ]
    );
    injected(got.error);

    clear_fault(&store);
    let (n, effects) = scan
        .scan(&store, "/srv/a", Some(&agents), 1.0, |_| None)
        .into_result()
        .unwrap();
    assert_eq!(n, 2);
    let notified: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Notify { title, .. } => Some(title.as_str()),
            Effect::Markers(_) => None,
        })
        .collect();
    assert_eq!(
        notified,
        ["A2 · w1:p2"],
        "the open alert is not notified again"
    );
}

#[test]
fn a_cleared_alert_keeps_its_markers_refresh_when_a_later_step_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    long_blocked(&store, "w1:p1");
    let agents = [blocked_agent("w1:p1")];
    let mut scan = BlockedScan::default();
    drop(scan.scan(&store, "/srv/a", Some(&agents), 0.0, |_| None));
    // the pane left the list: its alert clears, then the next step fails
    fault(
        &store,
        "UPDATE",
        "alerts",
        "NEW.closed_at IS NOT NULL AND NEW.status = 'acked'",
    );
    store
        .db()
        .execute(
            "INSERT INTO alerts (created_at, kind, severity, title, status, pane_id, herdr_server) \
             VALUES ('t', 'blocked', 'warning', 'Question Title 123', 'acked', 'w1:p9', '/srv/a')",
            [],
        )
        .unwrap();

    let got = scan.scan(&store, "/srv/a", Some(&[]), 1.0, |_| None);

    assert_eq!(got.value.1, [Effect::Markers(Some("w1:p1".into()))]);
    injected(got.error);
}

#[test]
fn herdr_not_answering_is_no_error_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let got = BlockedScan::default().scan(&store, "/srv/a", None, 0.0, |_| None);
    assert_eq!(got.into_result().unwrap(), (0, Vec::new()));
}

/// A machine the last pull reached.
fn reached(store: &Store) {
    store
        .db()
        .execute(
            "INSERT INTO remote_machines (machine_id, label, target, last_ok_at) \
             VALUES ('m1', 'box', 'box.lan', 't')",
            [],
        )
        .unwrap();
}

/// A queued action on question `remote_id`, in the mirror of no question.
fn queued(store: &Store, request: &str, remote_id: i64, expires: &str, attempts: i64) {
    store
        .db()
        .execute(
            "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at, \
             expires_at, state, attempts) VALUES (?, 'm1', 'dd', ?, 'k', 'answer', ?, ?, 'queued', ?)",
            params![request, remote_id, request, expires, attempts],
        )
        .unwrap();
}

fn state(store: &Store, request: &str) -> String {
    store
        .db()
        .query_row(
            "SELECT state FROM remote_outbox WHERE request_id = ?",
            [request],
            |r| r.get(0),
        )
        .unwrap()
}

const FAR: &str = "2999-01-01T00:00:00+00:00";
const PAST: &str = "2020-01-01T00:00:00+00:00";

#[test]
fn a_flush_keeps_the_notice_of_a_row_settled_before_a_later_rows_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    reached(&store);
    queued(&store, "r1", 1, FAR, 0);
    queued(&store, "r2", 2, FAR, 0);
    fault(&store, "UPDATE", "remote_outbox", "NEW.request_id = 'r2'");

    let got = store.flush("m1", &mut || at(0), &mut |_, _, _| {
        panic!("neither row fits the mirror: nothing is sent")
    });

    assert_eq!(titles(&got.value.notices), ["box:Q1: answer not sent"]);
    assert_eq!(got.value.aborted, None);
    injected(got.error);
    assert_eq!(state(&store, "r1"), "rejected");
    assert_eq!(state(&store, "r2"), "queued");
}

#[test]
fn a_flush_stopped_by_an_expired_rows_error_keeps_the_earlier_notice() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    reached(&store);
    queued(&store, "r1", 1, PAST, 0);
    queued(&store, "r2", 2, PAST, 0);
    fault(&store, "UPDATE", "remote_outbox", "NEW.request_id = 'r2'");

    let got = store.flush("m1", &mut || at(0), &mut |_, _, _| {
        panic!("a row never sent is not asked about")
    });

    assert_eq!(titles(&got.value.notices).len(), 1);
    injected(got.error);
    assert_eq!(state(&store, "r1"), "failed");
}

#[test]
fn a_flush_whose_first_read_fails_reports_the_error_with_nothing_collected() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    store
        .db()
        .execute_batch("ALTER TABLE remote_machines RENAME TO moved_away")
        .unwrap();
    let got = store.flush("m1", &mut || at(0), &mut |_, _, _| {
        panic!("nothing is sent")
    });
    assert!(got.value.notices.is_empty());
    assert!(got.error.is_some());
}

#[test]
fn a_sweep_keeps_the_notice_of_a_row_expired_before_a_later_rows_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    reached(&store);
    queued(&store, "r1", 1, PAST, 0);
    queued(&store, "r2", 2, PAST, 0);
    fault(&store, "UPDATE", "remote_outbox", "NEW.request_id = 'r2'");

    let got = store.expire_sweep(&mut || at(0));

    assert_eq!(got.value.len(), 1, "{:?}", got.value);
    injected(got.error);
    assert_eq!(state(&store, "r1"), "failed");
    assert_eq!(state(&store, "r2"), "queued");
}

const DB_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn export(new_id: i64) -> String {
    dumps(
        &json!({"version": "0.10.6", "format": 1, "db_id": DB_ID, "seq": 5, "full": false,
        "questions": [{"id": new_id, "qkey": "00000001", "group": 1, "status": "open",
                       "question": "Question Text 345", "options": ["Option Label 1"]}],
        "ids": [new_id], "alerts": []}),
    )
}

fn plugins() -> String {
    dumps(&json!({"result": {"plugins": [
        {"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "/r/root"}]}}))
}

fn answer(stdout: String, code: i32, stderr: &str) -> CallResult {
    CallResult {
        code,
        stdout,
        stderr: stderr.into(),
    }
}

/// The remote's replies: the paths probe, its plugin list, then `export`.
fn remote(command: &FixedCommand, stdin: Option<&str>, export: &CallResult) -> CallResult {
    match stdin {
        None if command.as_str() == PROBE => {
            answer("/r/bin/uv\n/r/bin/herdr\n/r/bin/claude\n".into(), 0, "")
        }
        None => answer(plugins(), 0, ""),
        Some(_) => export.clone(),
    }
}

/// An answered mirrored question `id` stuck on the remote.
fn stuck(store: &Store, id: i64) {
    let data = dumps(&json!({"id": id, "status": "answered", "agent_gone": true,
                             "question": "Question Text 345"}));
    store
        .db()
        .execute(
            "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
             VALUES ('m1', ?, ?, 'k', 'answered', ?, 't')",
            params![DB_ID, id, data],
        )
        .unwrap();
}

#[test]
fn a_pull_keeps_its_notices_when_a_stuck_answers_notice_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    store
        .record_machine("m1", Some("box"), Some("box.lan"))
        .unwrap();
    stuck(&store, 7);
    stuck(&store, 8);
    fault(
        &store,
        "UPDATE",
        "remote_questions",
        "NEW.remote_id = 8 AND NEW.issue_notified_at IS NOT NULL",
    );
    let exported = answer(export(1), 0, "");

    let got = store.pull(
        "m1",
        Some("box"),
        Some("box.lan"),
        &mut || at(0),
        &mut |_: &SafeTarget, c: &FixedCommand, i: Option<&str>| remote(c, i, &exported),
    );

    let (ok, notices) = got.value;
    assert!(!ok, "Python's raises before its flush");
    let titles = titles(&notices);
    assert_eq!(titles.len(), 2, "{titles:?}");
    assert!(titles[0].starts_with("box · Q1"), "{titles:?}");
    assert!(titles[1].starts_with("box:Q7 not delivered"), "{titles:?}");
    injected(got.error);

    // what was notified stays notified; the failed one is told on the next pass
    clear_fault(&store);
    let again = store
        .notify_problems("m1", "box", at(1))
        .into_result()
        .unwrap();
    let again = self::titles(&again).join(",");
    assert!(again.starts_with("box:Q8 not delivered"), "{again}");
    assert!(!again.contains("Q7"), "{again}");
}

fn backoff(store: &Store) -> Option<String> {
    store
        .db()
        .query_row(
            "SELECT backoff_until FROM remote_machines WHERE machine_id = 'm1'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// How the remote answers a call.
type Reply = Box<dyn Fn(&FixedCommand, Option<&str>) -> CallResult>;

/// A pull whose remote takes 10 s over each call, answering with `reply`.
fn pull_taking_10s(
    store: &Store,
    reply: &dyn Fn(&FixedCommand, Option<&str>) -> CallResult,
) -> (bool, Vec<Notice>) {
    let time = Cell::new(0);
    let got = store.pull(
        "m1",
        Some("box"),
        Some("box.lan"),
        &mut || at(time.get()),
        &mut |_: &SafeTarget, c: &FixedCommand, i: Option<&str>| {
            time.set(10);
            reply(c, i)
        },
    );
    assert!(got.error.is_none());
    got.value
}

#[test]
fn a_failed_pulls_backoff_counts_from_the_failure_whatever_failed() {
    // Python's `_fail` reads the clock after the call: a machine that times out 10 s in
    // must not have its first backoff already over
    let bad_format = dumps(&json!({"version": "0.10.6", "format": 99, "db_id": DB_ID, "seq": 5}));
    let failing_at_export =
        |export: CallResult| move |c: &FixedCommand, i: Option<&str>| remote(c, i, &export);
    let cases: Vec<(&str, Reply)> = vec![
        (
            "the paths probe",
            Box::new(|_, _| answer(String::new(), 255, "no route")),
        ),
        (
            "the export's exit",
            Box::new(failing_at_export(answer(String::new(), 124, "timed out"))),
        ),
        (
            "no export here",
            Box::new(failing_at_export(answer(String::new(), 2, ""))),
        ),
        (
            "a garbled export",
            Box::new(failing_at_export(answer("Question Text 345".into(), 0, ""))),
        ),
        (
            "another format",
            Box::new(failing_at_export(answer(bad_format, 0, ""))),
        ),
    ];
    for (what, reply) in cases {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert!(!pull_taking_10s(&store, &*reply).0, "{what}");
        assert_eq!(backoff(&store), Some(iso_seconds(at(15))), "{what}");
    }
}

#[test]
fn a_pull_that_worked_is_stamped_after_its_call() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let exported = answer(export(1), 0, "");
    let (ok, _) = pull_taking_10s(&store, &|c, i| remote(c, i, &exported));
    assert!(ok);
    let ok_at: String = store
        .db()
        .query_row(
            "SELECT last_ok_at FROM remote_machines WHERE machine_id = 'm1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ok_at, iso_seconds(at(10)));
}

#[test]
fn a_pull_in_backoff_reads_the_clock_and_calls_nobody() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let timed_out = answer(String::new(), 124, "timed out");
    let mut ssh = |_: &SafeTarget, c: &FixedCommand, i: Option<&str>| remote(c, i, &timed_out);
    drop(store.pull("m1", Some("box"), Some("box.lan"), &mut || at(0), &mut ssh));
    let got = store.pull(
        "m1",
        Some("box"),
        Some("box.lan"),
        &mut || at(4),
        &mut |_, _, _| panic!("in backoff: no call"),
    );
    assert!(!got.value.0);
    assert!(got.error.is_none());
    // past it, the machine is called again
    let calls = Cell::new(0);
    drop(store.pull(
        "m1",
        Some("box"),
        Some("box.lan"),
        &mut || at(5),
        &mut |_: &SafeTarget, c: &FixedCommand, i: Option<&str>| {
            calls.set(calls.get() + 1);
            remote(c, i, &timed_out)
        },
    ));
    assert!(calls.get() > 0);
}

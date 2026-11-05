use std::time::{Duration, UNIX_EPOCH};

use inbox_core::popup::{Detail, Effect, RemoteRef};
use serde_json::Value;

use super::*;
use crate::store::mirror::iso;
use crate::tui::mirror_rows::{alert, machine, outbox, question, queued, state};
use crate::tui::send::BUSY;

const NOW: u64 = 1_800_000_000;
const PAST: &str = "2020-01-01T00:00:00+00:00";
const FUTURE: &str = "2040-01-01T00:00:00+00:00";

fn env() -> RemoteEnv {
    RemoteEnv {
        expiry: Duration::from_secs(3 * 3600),
        wall: UNIX_EPOCH + Duration::from_secs(NOW),
    }
}

fn store() -> (tempfile::TempDir, std::path::PathBuf, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let s = Store::open_migrating(&path, Duration::from_millis(100)).unwrap();
    machine(&s, "m1", Some("box"));
    question(&s, "m1", 1, 1);
    alert(&s, "m1", 4);
    (dir, path, s)
}

fn shown_as(id: i64) -> RemoteRef {
    RemoteRef {
        machine_id: "m1".into(),
        remote_id: id,
        db_id: "dd".into(),
        qkey: format!("k{id}"),
    }
}

fn answer() -> Effect {
    Effect::RemoteAnswer {
        question: shown_as(1),
        detail: Detail {
            text: Some("Answer 123".into()),
            ..Detail::default()
        },
    }
}

fn ack() -> Effect {
    Effect::RemoteAck {
        machine_id: "m1".into(),
        alert: 4,
        db_id: "dd".into(),
        akey: "a4".into(),
    }
}

fn alert_status(s: &Store) -> String {
    s.db()
        .query_row("SELECT status FROM remote_alerts", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn an_answer_is_queued_under_the_identity_shown() {
    let (_d, _p, s) = store();
    assert_eq!(write(&s, &answer(), env()), Outcome::Done);
    let row = s.outbox_rows().unwrap().remove(0);
    assert_eq!(
        (
            row.machine_id.as_str(),
            row.remote_id,
            row.qkey.as_str(),
            row.db_id.as_str()
        ),
        ("m1", 1, "k1", "dd")
    );
    assert_eq!(
        (row.action.as_str(), row.state.as_str()),
        ("answer", "queued")
    );
    assert_eq!(row.request_id.len(), 32);
    let detail: Value = serde_json::from_str(row.detail_json.as_deref().unwrap()).unwrap();
    assert_eq!(
        detail,
        serde_json::json!({"choices": [], "notes": {}, "text": "Answer 123"})
    );
    assert_eq!(row.created_at, iso(env().wall));
    assert_eq!(row.expires_at, iso(env().wall + env().expiry));
}

#[test]
fn a_dismissal_is_queued_with_no_detail() {
    let (_d, _p, s) = store();
    let dismiss = Effect::RemoteDismiss {
        question: shown_as(1),
    };
    assert_eq!(write(&s, &dismiss, env()), Outcome::Done);
    let row = s.outbox_rows().unwrap().remove(0);
    assert_eq!((row.action.as_str(), row.detail_json), ("dismiss", None));
}

#[test]
fn two_actions_never_share_a_request_id() {
    let (_d, _p, s) = store();
    question(&s, "m1", 2, 2);
    let dismiss = |id| Effect::RemoteDismiss {
        question: shown_as(id),
    };
    assert_eq!(write(&s, &dismiss(1), env()), Outcome::Done);
    assert_eq!(write(&s, &dismiss(2), env()), Outcome::Done);
    assert_eq!(queued(&s).len(), 2);
}

#[test]
fn a_question_takes_one_action_however_stale_the_models_copy() {
    let refused = Outcome::Refused(Some(
        "not queued: that question already has an action queued".into(),
    ));
    let dismiss = Effect::RemoteDismiss {
        question: shown_as(1),
    };
    for state in ["queued", "done"] {
        let (_d, _p, s) = store();
        outbox(&s, "r1", "m1", state, FUTURE);
        assert_eq!(write(&s, &answer(), env()), refused, "{state}");
        assert_eq!(write(&s, &dismiss, env()), refused, "{state}");
        assert_eq!(queued(&s).len(), 1);
    }
}

#[test]
fn an_action_that_ended_or_another_question_does_not_count() {
    for state in ["failed", "rejected", "seen"] {
        let (_d, _p, s) = store();
        outbox(&s, "r1", "m1", state, FUTURE);
        assert_eq!(write(&s, &answer(), env()), Outcome::Done, "{state}");
    }
    // an acknowledgement, and a question that had this number before a remote reset
    let (_d, _p, s) = store();
    outbox(&s, "r1", "m1", "queued", FUTURE);
    s.db()
        .execute("UPDATE remote_outbox SET action = 'ack_alert'", [])
        .unwrap();
    assert_eq!(write(&s, &answer(), env()), Outcome::Done);
    let (_d, _p, s) = store();
    outbox(&s, "r1", "m1", "queued", FUTURE);
    s.db()
        .execute("UPDATE remote_outbox SET qkey = 'old'", [])
        .unwrap();
    assert_eq!(write(&s, &answer(), env()), Outcome::Done);
}

#[test]
fn looking_and_queueing_happen_under_the_write_lock() {
    // the row already queued would refuse without a word from the database: the lock comes first
    let (_d, path, s) = store();
    outbox(&s, "r1", "m1", "queued", FUTURE);
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        write(&s, &answer(), env()),
        Outcome::Refused(Some(BUSY.to_string()))
    );
    holder.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn nothing_is_queued_under_a_question_the_mirror_replaced() {
    for (set, why) in [
        (
            "qkey = 'other'",
            "not queued: that question changed — reopen it",
        ),
        (
            "db_id = 'other'",
            "not queued: that question changed — reopen it",
        ),
    ] {
        let (_d, _p, s) = store();
        s.db()
            .execute(&format!("UPDATE remote_questions SET {set}"), [])
            .unwrap();
        for effect in [
            answer(),
            Effect::RemoteDismiss {
                question: shown_as(1),
            },
        ] {
            assert_eq!(
                write(&s, &effect, env()),
                Outcome::Refused(Some(why.into())),
                "{set}"
            );
        }
        assert_eq!(queued(&s), []);
    }
}

#[test]
fn nothing_is_queued_for_a_question_the_mirror_lost() {
    let (_d, _p, s) = store();
    s.db().execute("DELETE FROM remote_questions", []).unwrap();
    assert_eq!(
        write(&s, &answer(), env()),
        Outcome::Refused(Some("not queued: that remote question is gone".into()))
    );
    assert_eq!(queued(&s), []);
}

#[test]
fn another_machines_question_is_not_taken_for_it() {
    let (_d, _p, s) = store();
    machine(&s, "m2", None);
    question(&s, "m2", 2, 2);
    let effect = Effect::RemoteDismiss {
        question: shown_as(2), // Q2 is on m2, not m1
    };
    assert!(matches!(write(&s, &effect, env()), Outcome::Refused(_)));
    assert_eq!(queued(&s), []);
}

#[test]
fn an_ack_queues_and_shows_the_alert_acknowledged() {
    let (_d, _p, s) = store();
    assert_eq!(write(&s, &ack(), env()), Outcome::Done);
    assert_eq!(
        queued(&s),
        [(
            "ack_alert".to_string(),
            4,
            "a4".to_string(),
            "dd".to_string()
        )]
    );
    assert_eq!(alert_status(&s), "acked");
}

#[test]
fn an_ack_lands_whole_or_not_at_all() {
    let (_d, _p, s) = store();
    s.db()
        .execute_batch(
            "CREATE TRIGGER no_ack BEFORE UPDATE ON remote_alerts \
             BEGIN SELECT RAISE(ABORT, 'no'); END;",
        )
        .unwrap();
    assert!(matches!(
        write(&s, &ack(), env()),
        Outcome::Refused(Some(_))
    ));
    assert_eq!(queued(&s), [], "no outbox row without the mirror's acked");
    assert_eq!(alert_status(&s), "open");
}

#[test]
fn an_ack_of_an_alert_the_mirror_replaced_changes_nothing() {
    let (_d, _p, s) = store();
    s.db()
        .execute("UPDATE remote_alerts SET akey = 'other'", [])
        .unwrap();
    assert_eq!(
        write(&s, &ack(), env()),
        Outcome::Refused(Some(
            "that alert changed or is gone — reopen the inbox".into()
        ))
    );
    assert_eq!(queued(&s), []);
    assert_eq!(alert_status(&s), "open");
}

fn clear(id: &str, expire: bool) -> Effect {
    Effect::ClearOutbox {
        request_id: id.into(),
        expire,
    }
}

#[test]
fn clearing_a_failed_or_rejected_row_marks_it_seen() {
    for from in ["failed", "rejected"] {
        let (_d, _p, s) = store();
        outbox(&s, "r1", "m1", from, FUTURE);
        assert_eq!(write(&s, &clear("r1", false), env()), Outcome::Done);
        assert_eq!(state(&s, "r1").0, "seen");
    }
}

#[test]
fn clearing_an_expired_row_settles_it_first() {
    let (_d, _p, s) = store();
    outbox(&s, "r1", "m1", "queued", PAST);
    assert_eq!(write(&s, &clear("r1", true), env()), Outcome::Done);
    assert_eq!(state(&s, "r1"), ("seen".into(), Some("expired".into())));
}

#[test]
fn a_row_that_started_sending_since_the_read_is_left_alone() {
    let (_d, _p, s) = store();
    outbox(&s, "r1", "m1", "queued", PAST);
    s.db()
        .execute(
            "UPDATE remote_outbox SET sending_at = ?",
            [iso(env().wall - Duration::from_secs(1))],
        )
        .unwrap();
    assert_eq!(write(&s, &clear("r1", true), env()), Outcome::Refused(None));
    assert_eq!(state(&s, "r1"), ("queued".into(), None));
}

#[test]
fn a_row_not_clearable_any_more_is_refused_without_a_word() {
    let (_d, _p, s) = store();
    outbox(&s, "not-yet", "m1", "queued", FUTURE);
    outbox(&s, "done", "m1", "done", PAST);
    for (id, was) in [("not-yet", "queued"), ("done", "done")] {
        assert_eq!(write(&s, &clear(id, true), env()), Outcome::Refused(None));
        assert_eq!(state(&s, id).0, was);
    }
    assert_eq!(
        write(&s, &clear("gone", false), env()),
        Outcome::Refused(None)
    );
}

#[test]
fn the_look_at_a_row_happens_under_the_write_lock() {
    // not clearable: without the lock this would answer at once, and a row could start
    // sending between the look and the write
    let (_d, path, s) = store();
    outbox(&s, "r1", "m1", "queued", FUTURE);
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        write(&s, &clear("r1", true), env()),
        Outcome::Refused(Some(BUSY.to_string()))
    );
    holder.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn whether_a_row_expired_is_asked_of_the_time_given() {
    let (_d, _p, s) = store();
    outbox(&s, "r1", "m1", "queued", PAST);
    let before = RemoteEnv {
        wall: UNIX_EPOCH + Duration::from_secs(1_500_000_000), // 2017, before PAST
        ..env()
    };
    assert_eq!(
        write(&s, &clear("r1", true), before),
        Outcome::Refused(None)
    );
    assert_eq!(state(&s, "r1").0, "queued");
}

#[test]
fn clearing_a_problem_stamps_it() {
    let (_d, _p, s) = store();
    let effect = Effect::ClearProblem {
        machine_id: "m1".into(),
        remote_id: 1,
    };
    assert_eq!(write(&s, &effect, env()), Outcome::Done);
    let at: Option<String> = s
        .db()
        .query_row("SELECT cleared_at FROM remote_questions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(at, Some(iso(env().wall)));
}

#[test]
fn a_busy_database_refuses_every_remote_write_with_the_notice() {
    let (_d, path, s) = store();
    outbox(&s, "r1", "m1", "failed", FUTURE);
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    for effect in [
        answer(),
        Effect::RemoteDismiss {
            question: shown_as(1),
        },
        ack(),
        clear("r1", false),
        Effect::ClearProblem {
            machine_id: "m1".into(),
            remote_id: 1,
        },
    ] {
        assert_eq!(
            write(&s, &effect, env()),
            Outcome::Refused(Some(BUSY.to_string())),
            "{effect:?}"
        );
    }
    holder.execute_batch("ROLLBACK").unwrap();
    assert_eq!(state(&s, "r1").0, "failed");
    assert_eq!(alert_status(&s), "open");
}

#[test]
fn another_effect_is_no_write_here() {
    let (_d, _p, s) = store();
    assert_eq!(write(&s, &Effect::Exit, env()), Outcome::Done);
    assert_eq!(queued(&s), []);
}

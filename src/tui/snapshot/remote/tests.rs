use std::time::{Duration, UNIX_EPOCH};

use inbox_core::popup::RemoteRef;
use serde_json::json;

use super::*;
use crate::clock::local_hhmm;
use crate::tui::mirror_rows::{self as rows, SEEN, machine, outbox};

const NOW: u64 = 1_800_000_000; // 2027-01-15T08:00:00Z
const PAST: &str = "2020-01-01T00:00:00+00:00";
const FUTURE: &str = "2040-01-01T00:00:00+00:00";

fn wall() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(NOW)
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    (dir, s)
}

#[test]
fn an_empty_mirror_reads_empty() {
    let (_d, s) = store();
    assert_eq!(read(&s, wall()).unwrap(), Remote::default());
}

#[test]
fn a_question_reads_as_the_views_show_it() {
    let (_d, s) = store();
    machine(&s, "m1", Some("box"));
    let data = json!({"id": 3, "group": 2, "qkey": "k3", "status": "open", "pane_id": "w1:p1",
        "question": "Question Title 123", "title": "Group Title 123", "header": "Head 123",
        "tab_label": "tab", "asked_by": "agent", "created_at": SEEN, "multi_select": true,
        "options": [{"label": "a", "description": "d", "preview": "p"}, {"label": "b"}]});
    s.db()
        .execute(
            "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
             VALUES ('m1', 'dd', 3, 'k3', 'open', ?, ?)",
            [data.to_string(), SEEN.to_string()],
        )
        .unwrap();
    let r = read(&s, wall()).unwrap();
    let g = r.group("m1|dd|2").unwrap();
    assert_eq!(g.machine_id, "m1");
    let rq = &g.questions[0];
    assert_eq!(
        rq.r#ref(),
        RemoteRef {
            machine_id: "m1".into(),
            remote_id: 3,
            db_id: "dd".into(),
            qkey: "k3".into()
        }
    );
    let q = &rq.question;
    assert_eq!((q.id, q.group, q.status.as_str()), (3, 2, "open"));
    assert_eq!(q.question, "Question Title 123");
    assert_eq!(q.group_title.as_deref(), Some("Group Title 123"));
    assert_eq!(q.header.as_deref(), Some("Head 123"));
    assert_eq!(q.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(q.tab_label.as_deref(), Some("tab"));
    assert_eq!(q.asked_by.as_deref(), Some("agent"));
    assert_eq!(q.created_at, SEEN);
    assert!(q.multi_select);
    assert_eq!(q.options.len(), 2);
    assert_eq!(q.options[0].description.as_deref(), Some("d"));
    assert_eq!(q.options[0].preview.as_deref(), Some("p"));
    assert_eq!(q.draft, None);
}

#[test]
fn a_question_with_an_action_queued_is_left_out_but_stays_in_the_mirror() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    rows::question(&s, "m1", 1, 1);
    rows::question(&s, "m1", 2, 1);
    outbox(&s, "r1", "m1", "queued", FUTURE);
    let r = read(&s, wall()).unwrap();
    let ids: Vec<i64> = r.groups[0]
        .questions
        .iter()
        .map(|q| q.question.id)
        .collect();
    assert_eq!(ids, [2]);
    let mirrored: Vec<i64> = r.mirror.iter().map(|m| m.remote_id).collect();
    assert_eq!(mirrored, [1, 2]);
}

#[test]
fn a_closed_question_stays_in_the_mirror_only() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    rows::question(&s, "m1", 1, 1);
    s.db()
        .execute("UPDATE remote_questions SET status = 'answered'", [])
        .unwrap();
    let r = read(&s, wall()).unwrap();
    assert_eq!((r.groups.len(), r.mirror.len()), (0, 1));
}

#[test]
fn an_alert_reads_with_its_identity_and_pythons_defaults() {
    let (_d, s) = store();
    machine(&s, "m1", Some("box"));
    rows::alert(&s, "m1", 4);
    s.db()
        .execute(
            "INSERT INTO remote_alerts (machine_id, db_id, alert_id, akey, status, data_json, first_seen_at) \
             VALUES ('m1', 'dd', 5, 'a5', 'open', '{\"id\": 5, \"akey\": \"a5\"}', ?)",
            [SEEN],
        )
        .unwrap();
    let r = read(&s, wall()).unwrap();
    let a = r.alert("m1|4").unwrap();
    assert_eq!((a.db_id.as_str(), a.akey.as_str()), ("dd", "a4"));
    assert_eq!(
        (
            a.alert.kind.as_str(),
            a.alert.severity.as_str(),
            a.alert.title.as_str()
        ),
        ("blocked", "urgent", "Alert Title 123")
    );
    let bare = &r.alert("m1|5").unwrap().alert;
    assert_eq!(
        (
            bare.kind.as_str(),
            bare.severity.as_str(),
            bare.status.as_str()
        ),
        ("agent", "warning", "open")
    );
}

#[test]
fn an_acked_alert_is_not_read() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    rows::alert(&s, "m1", 4);
    s.db()
        .execute("UPDATE remote_alerts SET status = 'acked'", [])
        .unwrap();
    assert_eq!(read(&s, wall()).unwrap().alerts, []);
}

#[test]
fn outbox_rows_carry_the_answers_the_clock_gives() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    outbox(&s, "waiting", "m1", "queued", FUTURE);
    outbox(&s, "expired", "m1", "queued", PAST);
    outbox(&s, "sending", "m1", "queued", PAST);
    outbox(&s, "failed", "m1", "failed", FUTURE);
    outbox(&s, "seen", "m1", "seen", PAST);
    s.db()
        .execute(
            "UPDATE remote_outbox SET sending_at = ? WHERE request_id = 'sending'",
            [iso(wall())],
        )
        .unwrap();
    s.db()
        .execute(
            "UPDATE remote_outbox SET result = 'Reason 123' WHERE request_id = 'failed'",
            [],
        )
        .unwrap();
    let r = read(&s, wall()).unwrap();
    let flags = |id: &str| {
        let o = r.outbox_row(id).unwrap();
        (o.sending, o.expired, o.clearable)
    };
    assert_eq!(flags("waiting"), (false, false, false));
    assert_eq!(flags("expired"), (false, true, true));
    assert_eq!(flags("sending"), (true, true, false));
    assert_eq!(flags("failed"), (false, false, true));
    assert!(r.outbox_row("seen").is_none(), "cleared rows are not shown");
    let failed = r.outbox_row("failed").unwrap();
    assert_eq!(failed.result.as_deref(), Some("Reason 123"));
    assert_eq!(
        r.outbox_row("waiting").unwrap().expires,
        local_hhmm(unix_seconds(FUTURE).unwrap())
    );
}

#[test]
fn an_outbox_row_expires_at_its_second() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    outbox(&s, "r1", "m1", "queued", &iso(wall()));
    let before = wall() - Duration::from_secs(1);
    assert!(!read(&s, before).unwrap().outbox[0].expired);
    assert!(read(&s, wall()).unwrap().outbox[0].expired);
}

#[test]
fn a_stuck_answer_reads_as_a_problem_until_cleared() {
    let (_d, s) = store();
    machine(&s, "m1", Some("box"));
    let data = json!({"id": 9, "group": 9, "qkey": "k9", "status": "answered", "agent_gone": true,
        "question": "Question Title 123", "answer": "Answer 123"});
    s.db()
        .execute(
            "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
             VALUES ('m1', 'dd', 9, 'k9', 'answered', ?, ?)",
            [data.to_string(), SEEN.to_string()],
        )
        .unwrap();
    let r = read(&s, wall()).unwrap();
    let p = r.problem("m1|9").unwrap();
    assert_eq!(p.label, "box");
    assert_eq!(p.why, "agent closed — answer kept for check_answers");
    assert_eq!(p.question, "Question Title 123");
    assert_eq!(p.answer.as_deref(), Some("Answer 123"));
    s.clear_problem("m1", 9, wall()).unwrap();
    assert_eq!(read(&s, wall()).unwrap().problems, []);
}

#[test]
fn a_machine_reads_with_its_offline_note() {
    let (_d, s) = store();
    machine(&s, "m1", Some("box"));
    machine(&s, "m2", None);
    s.db()
        .execute(
            "UPDATE remote_machines SET last_error = 'Reason 123' WHERE machine_id = 'm2'",
            [],
        )
        .unwrap();
    let r = read(&s, wall()).unwrap();
    assert_eq!(r.machine("m1").unwrap().offline, None);
    assert_eq!(r.machine("m1").unwrap().label.as_deref(), Some("box"));
    assert_eq!(
        r.machine("m2").unwrap().offline.as_deref(),
        Some("not reached yet")
    );
}

#[test]
fn a_mirrored_row_that_is_not_json_fails_the_read() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    rows::question(&s, "m1", 1, 1);
    s.db()
        .execute("UPDATE remote_questions SET data_json = 'x'", [])
        .unwrap();
    assert!(read(&s, wall()).is_err());
}

#[test]
fn a_row_without_an_id_is_left_out() {
    let (_d, s) = store();
    machine(&s, "m1", None);
    let map: Map<String, Value> = Map::new();
    assert_eq!(question("m1", &map), None);
    assert_eq!(alert("m1", &map), None);
}

//! The scenarios of `tools/popup_remote_screens.py` (popup-remote.json): a
//! remote machine's rows from the local mirror, its question and alerts
//! opened, the confirmation and the placeholders.

use inbox_core::popup::{
    Alert, Machine, OutboxRow, Problem, QOption, Question, RemoteAlert, RemoteGroup,
    RemoteQuestion, RemoteRef, Snapshot,
};

use super::base::Shots;
use super::data::{BILLING, HELD, iso, opts, question, snapshot};
use super::drive::Pilot;

const FILE: &str = "popup-remote";
const DB: &str = "dddddddddddddddddddddddddddddddd";

/// `popup_remote.rq`: a remote question asked from tab-A over there.
pub fn rq(mid: &str, id: i64, group: i64, text: &str, options: Vec<QOption>) -> RemoteQuestion {
    RemoteQuestion {
        machine_id: mid.into(),
        db_id: DB.into(),
        qkey: format!("{id:02}").repeat(8),
        question: Question {
            id,
            group,
            status: "open".into(),
            pane_id: Some("w1:p1".into()),
            tab_label: Some("tab-A".into()),
            options,
            question: text.into(),
            created_at: iso(HELD - 300 * id),
            ..Question::default()
        },
    }
}

pub fn group(qs: Vec<RemoteQuestion>) -> RemoteGroup {
    let q = &qs[0];
    RemoteGroup {
        key: format!("{}|{DB}|{}", q.machine_id, q.question.group),
        machine_id: q.machine_id.clone(),
        questions: qs,
    }
}

/// `popup_remote.ralert`.
fn ralert(id: i64, title: &str, kind: &str) -> RemoteAlert {
    RemoteAlert {
        machine_id: "m1".into(),
        db_id: DB.into(),
        akey: format!("a{id}").repeat(8),
        alert: Alert {
            id,
            kind: kind.into(),
            status: "open".into(),
            pane_id: Some("w1:p1".into()),
            severity: "warning".into(),
            title: title.into(),
            tab_label: Some("tab-A".into()),
            created_at: iso(HELD - 3600 * id),
            ..Alert::default()
        },
    }
}

fn outbox(n: u32, rid: i64, action: &str, state: &str) -> OutboxRow {
    OutboxRow {
        request_id: format!("{n:032x}"),
        machine_id: "m1".into(),
        action: action.into(),
        remote_id: rid,
        state: state.into(),
        expires: "15:30".into(),
        ..OutboxRow::default()
    }
}

fn mirror(qs: &[&RemoteQuestion]) -> Vec<RemoteRef> {
    qs.iter().map(|q| q.r#ref()).collect()
}

fn local() -> Question {
    let text = "Publish the v2 docs now or after the release?";
    question(1, 1, BILLING, text, opts(&["now", "after"]), "")
}

/// The mirror of the main scenario: project1 offline, two alerts, a group
/// of two, a lone group, four outbox rows, an undeliverable answer.
fn mirrored() -> Snapshot {
    let mut q1 = local();
    q1.header = Some("Rollout".into());
    let mut s = snapshot(vec![q1], vec![], vec![]);
    let yes_no = || opts(&["yes", "no"]);
    let mut q3 = rq("m1", 3, 3, "Ship the release today?", yes_no());
    let mut q4 = rq("m1", 4, 3, "Tag it as v2.0?", yes_no());
    for (q, header) in [(&mut q3, "Ship"), (&mut q4, "Tag")] {
        q.question.header = Some(header.into());
        q.question.tab_label = Some("Release".into());
        q.question.asked_by = Some("releaser".into());
    }
    let others: Vec<RemoteQuestion> = [(5, "Rotate the keys?"), (6, "Drop the cache?")]
        .into_iter()
        .chain([(7, "Bump the quota?"), (8, "Purge logs?")])
        .map(|(id, text)| rq("m1", id, id, text, yes_no()))
        .collect();
    let r = &mut s.remote;
    r.machines.push(Machine {
        machine_id: "m1".into(),
        label: Some("project1".into()),
        offline: Some("offline since 10:00 (2 h)".into()),
    });
    r.alerts = vec![
        ralert(1, "Deploy needs your eyes", "agent"),
        ralert(2, "Approval prompt", "blocked"),
    ];
    let mut all = vec![&q3, &q4];
    all.extend(others.iter());
    r.mirror = mirror(&all);
    r.groups = vec![
        group(vec![q3.clone(), q4.clone()]),
        group(vec![others[1].clone()]),
    ];
    let mut rejected = outbox(2, 6, "hide", "rejected");
    rejected.result = Some("answered with no".into());
    let mut expired = outbox(3, 7, "answer", "queued");
    expired.expired = true;
    let mut sending = outbox(4, 8, "dismiss", "queued");
    sending.sending = true;
    r.outbox = vec![outbox(1, 5, "answer", "queued"), rejected, expired, sending];
    r.problems.push(Problem {
        machine_id: "m1".into(),
        remote_id: 9,
        label: "project1".into(),
        why: "agent closed — answer kept for check_answers".into(),
        question: "Restart the worker?".into(),
        answer: Some("yes".into()),
    });
    s
}

pub fn screens(out: &mut Shots) {
    let size = (110, 30);
    let mut shot = |name: &str, p: &mut Pilot| {
        out.insert((FILE, name.to_string()), p.screen());
    };

    let mut now = mirrored();
    let mut p = Pilot::open(now.clone(), size);
    shot("remote rows", &mut p);
    p.press(&["down"]);
    shot("cursor on a remote group", &mut p);
    p.press(&["down"; 6]);
    shot("cursor on an undeliverable remote answer", &mut p);
    let mut keys = vec!["up"; 6];
    keys.push("enter");
    p.press(&keys);
    shot("remote question opened", &mut p);
    p.press(&["down", "n"]);
    p.type_text("soon");
    shot("a note on a remote question", &mut p);
    p.press(&["escape", "escape", "d"]);
    shot("remote dismiss confirmation", &mut p);
    p.press(&["n", "up", "up", "up", "enter"]);
    shot("remote alert opened", &mut p);
    p.press(&["escape", "down", "enter"]);
    shot("blocked remote alert opened", &mut p);
    p.press(&["escape", "down", "down", "enter", "enter"]);
    shot("first of two answered, the row stays", &mut p);
    // the database after it: Q3's answer queued (its own row), Q4 left in the group
    let r = &mut now.remote;
    r.groups[0].questions.remove(0);
    let mut queued = outbox(5, 3, "answer", "queued");
    queued.expires = "12:00".into();
    r.outbox.push(queued);
    p.poll(now.clone());
    shot("after a poll", &mut p);
    p.press(&["enter"]);
    now.remote.groups.remove(0); // Q4 withdrawn over there
    p.poll(now.clone());
    shot("remote question closed there", &mut p);
    p.press(&["escape", "up", "up", "up", "d"]);
    shot("remote alert acknowledged in the list", &mut p);

    let mut s = snapshot(vec![local()], vec![], vec![]);
    let mut q = rq("m2", 1, 1, "Ship it?", opts(&["yes"]));
    q.question.group_title = Some("Release train".into());
    s.remote.machines.push(Machine {
        machine_id: "m2".into(),
        ..Machine::default()
    });
    s.remote.mirror = mirror(&[&q]);
    s.remote.groups.push(group(vec![q]));
    let mut p = Pilot::open(s, size);
    p.press(&["down", "enter"]);
    shot("an unlabelled machine's question", &mut p);
    p.press(&["enter"]);
    shot("an unlabelled machine's row answered", &mut p);
}

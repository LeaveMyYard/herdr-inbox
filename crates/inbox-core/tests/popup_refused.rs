//! [`Event::RemoteRefused`]: a remote write the IO layer refused after the
//! model moved on. No Python scenario reaches it (Python writes first), so
//! these are written by hand. The rule: the popup ends up as if the model had
//! refused by itself.

use inbox_core::popup::{
    Alert, Detail, Effect, Entry, Event, Key, Machine, Mode, Open, OutboxRow, Popup, Problem,
    QOption, Question, Remote, RemoteAlert, RemoteGroup, RemoteQuestion, Severity, Snapshot, View,
};

const CHANGED: &str = "not queued: that question changed — reopen it";

fn rq(id: i64) -> RemoteQuestion {
    RemoteQuestion {
        machine_id: "m1".into(),
        db_id: "dd".into(),
        qkey: format!("k{id}"),
        question: Question {
            id,
            group: 7,
            status: "open".into(),
            options: vec![QOption::label("a")],
            question: format!("Question Title {id}"),
            ..Question::default()
        },
    }
}

/// The mirror with `ids` open in one group.
fn mirror(ids: &[i64]) -> Remote {
    let mut r = Remote::default();
    r.machines.push(Machine {
        machine_id: "m1".into(),
        label: Some("box".into()),
        offline: None,
    });
    r.mirror = ids.iter().map(|i| rq(*i).r#ref()).collect();
    if !ids.is_empty() {
        r.groups.push(RemoteGroup {
            key: "m1|dd|7".into(),
            machine_id: "m1".into(),
            questions: ids.iter().map(|i| rq(*i)).collect(),
        });
    }
    r
}

fn snapshot(remote: Remote) -> Snapshot {
    let mut s = Snapshot::default();
    s.remote = remote;
    s
}

struct Run {
    popup: Popup,
    now: u64,
}

impl Run {
    fn open(remote: Remote) -> Run {
        let (popup, _) = Popup::open(
            Open {
                snapshot: snapshot(remote),
                ..Open::default()
            },
            10_000,
        );
        Run { popup, now: 10_000 }
    }

    fn key(&mut self, name: &str) -> Vec<Effect> {
        self.now += 5_000;
        self.popup.handle(Event::Key(Key::parse(name)), self.now)
    }

    fn refused(&mut self, notice: Option<&str>, snap: Option<Snapshot>) -> Vec<Effect> {
        self.now += 1;
        let event = Event::RemoteRefused {
            notice: notice.map(str::to_string),
            snapshot: snap,
        };
        self.popup.handle(event, self.now)
    }

    /// The row's text as the model keeps it: closed for what reason, if any.
    fn closed(&self, entry: &Entry) -> Option<String> {
        self.popup.closed().get(entry).cloned()
    }
}

fn warning(text: &str) -> Effect {
    Effect::Notify {
        text: text.to_string(),
        severity: Severity::Warning,
    }
}

fn group_entry() -> Entry {
    Entry::Remote("m1|dd|7".into())
}

#[test]
fn a_refused_answer_goes_back_to_the_view_with_what_was_typed() {
    let mut r = Run::open(mirror(&[1, 2]));
    r.key("enter"); // the row
    assert_eq!(r.popup.mode(), Mode::Remote);
    r.key("down");
    r.key("enter"); // "Type something."
    for c in "Answer 123".chars() {
        r.key(&c.to_string());
    }
    let fx = r.key("enter");
    let detail = Detail {
        text: Some("Answer 123".into()),
        ..Detail::default()
    };
    let queued = Effect::RemoteAnswer {
        question: rq(1).r#ref(),
        detail,
    };
    assert_eq!(fx[0], queued);
    assert_eq!(r.popup.mode(), Mode::List, "the model moved on");

    let fx = r.refused(Some(CHANGED), None);

    assert_eq!(fx, [warning(CHANGED)]);
    assert_eq!(r.popup.mode(), Mode::Remote, "the view is back");
    assert_eq!(r.popup.remote_question().unwrap().0.question.id, 1);
    assert!(r.popup.data().remote.group("m1|dd|7").is_some());
    // what was typed is still there: Enter on the text row brings it back, Enter sends it
    r.key("enter");
    let View::Question(v) = r.popup.view() else {
        panic!("not a question view")
    };
    assert!(v.editing.is_some());
    assert_eq!(r.key("enter")[0], queued);
}

#[test]
fn a_refused_answer_undoes_the_placeholder_and_the_end_of_the_popup() {
    // the row's only question and the list's only row: the answer closes the row and the popup
    let mut r = Run::open(mirror(&[1]));
    r.key("enter");
    let fx = r.key("enter");
    assert!(fx.contains(&Effect::Quit), "{fx:?}");
    assert_eq!(
        r.closed(&group_entry()).as_deref(),
        Some("answered · sending to box")
    );

    assert_eq!(r.refused(Some(CHANGED), None), [warning(CHANGED)]);

    assert_eq!(r.closed(&group_entry()), None);
    assert_eq!(r.popup.mode(), Mode::Remote);
    assert!(
        matches!(r.key("enter")[0], Effect::RemoteAnswer { .. }),
        "the popup goes on"
    );
}

#[test]
fn the_database_after_a_refusal_replaces_the_copy_from_before() {
    let mut r = Run::open(mirror(&[1, 2]));
    r.key("enter");
    r.key("enter");
    r.refused(None, Some(snapshot(mirror(&[2]))));
    let ids: Vec<i64> = r.popup.data().remote.groups[0]
        .questions
        .iter()
        .map(|q| q.question.id)
        .collect();
    assert_eq!(ids, [2]);
    assert_eq!(
        r.popup.remote_question().unwrap().0.question.id,
        1,
        "the question on screen stays what it was"
    );
}

#[test]
fn a_refused_dismissal_keeps_the_row_open_on_the_questions_left() {
    let mut r = Run::open(mirror(&[1, 2, 3]));
    r.key("d");
    let fx = r.key("y");
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::RemoteDismiss { .. }))
            .count(),
        3
    );
    assert_eq!(
        r.closed(&group_entry()).as_deref(),
        Some("dismissing on box")
    );

    // Q1 was queued, Q2 refused: the read leaves Q2 and Q3 (Q1 is no longer answerable)
    let fx = r.refused(Some(CHANGED), Some(snapshot(mirror(&[2, 3]))));

    assert_eq!(fx, [warning(CHANGED)]);
    assert_eq!(r.closed(&group_entry()), None, "the row stays open");
    assert_eq!(r.popup.mode(), Mode::List);
    assert!(r.popup.confirm().is_none(), "y is not asked for again");
    r.key("d");
    let asked = r.popup.confirm().unwrap().remote().unwrap();
    let ids: Vec<i64> = asked.iter().map(|q| q.question.id).collect();
    assert_eq!(ids, [2, 3], "only what is left");
}

fn with_alert() -> Remote {
    let mut r = mirror(&[]);
    r.alerts.push(RemoteAlert {
        machine_id: "m1".into(),
        db_id: "dd".into(),
        akey: "ak".into(),
        alert: Alert {
            id: 4,
            kind: "agent".into(),
            status: "open".into(),
            severity: "warning".into(),
            title: "Alert Title 123".into(),
            ..Alert::default()
        },
    });
    r.groups = mirror(&[1]).groups; // a row besides it: the popup stays open
    r.mirror = mirror(&[1]).mirror;
    r
}

#[test]
fn a_refused_ack_in_the_list_keeps_the_alert_open() {
    let mut r = Run::open(with_alert());
    let entry = Entry::RemoteAlert("m1|4".into());
    r.key("up"); // the cursor starts on the question
    assert!(matches!(r.key("d")[0], Effect::RemoteAck { .. }));
    assert_eq!(r.closed(&entry).as_deref(), Some("acknowledged"));

    let why = "that alert changed or is gone — reopen the inbox";
    assert_eq!(r.refused(Some(why), None), [warning(why)]);

    assert_eq!(r.closed(&entry), None);
    assert!(r.popup.data().remote.alert("m1|4").is_some());
}

#[test]
fn a_refused_ack_in_its_view_goes_back_to_the_view() {
    let mut r = Run::open(with_alert());
    r.key("up");
    r.key("enter");
    assert_eq!(r.popup.mode(), Mode::Alert);
    assert!(matches!(r.key("d")[0], Effect::RemoteAck { .. }));
    assert_eq!(r.popup.mode(), Mode::List);

    r.refused(Some("why"), None);

    assert_eq!(r.popup.mode(), Mode::Alert);
    assert_eq!(r.popup.remote_alert().unwrap().alert.id, 4);
}

fn outbox_row() -> OutboxRow {
    OutboxRow {
        request_id: "r1".into(),
        machine_id: "m1".into(),
        action: "answer".into(),
        remote_id: 1,
        state: "queued".into(),
        expired: true,
        clearable: true,
        ..OutboxRow::default()
    }
}

#[test]
fn an_outbox_row_that_started_sending_stays_and_nothing_is_said() {
    let mut remote = mirror(&[]);
    remote.outbox.push(outbox_row());
    let mut r = Run::open(remote);
    let entry = Entry::Outbox("r1".into());
    let fx = r.key("d");
    let clear = Effect::ClearOutbox {
        request_id: "r1".into(),
        expire: true,
    };
    assert_eq!(fx, [clear]);
    assert_eq!(r.closed(&entry).as_deref(), Some("cleared"));

    let mut now = mirror(&[]);
    now.outbox.push(OutboxRow {
        sending: true,
        clearable: false,
        ..outbox_row()
    });
    assert_eq!(r.refused(None, Some(snapshot(now))), []);

    assert_eq!(r.closed(&entry), None);
    assert!(r.popup.data().remote.outbox_row("r1").unwrap().sending);
    assert_eq!(r.key("d"), [], "not clearable now: nothing happens");
}

#[test]
fn a_problem_the_database_would_not_clear_stays() {
    let mut remote = mirror(&[]);
    remote.problems.push(Problem {
        machine_id: "m1".into(),
        remote_id: 9,
        label: "box".into(),
        why: "agent closed — answer kept for check_answers".into(),
        ..Problem::default()
    });
    let mut r = Run::open(remote);
    let entry = Entry::Problem("m1|9".into());
    assert!(matches!(r.key("d")[0], Effect::ClearProblem { .. }));
    assert_eq!(r.closed(&entry).as_deref(), Some("cleared"));

    assert_eq!(r.refused(Some("busy"), None), [warning("busy")]);

    assert_eq!(r.closed(&entry), None);
    assert!(r.popup.data().remote.problem("m1|9").is_some());
}

#[test]
fn a_refusal_only_answers_the_action_just_taken() {
    let mut r = Run::open(mirror(&[1, 2]));
    r.key("enter");
    r.key("enter");
    assert_eq!(r.popup.mode(), Mode::List);
    r.now += 1;
    r.popup.handle(Event::Data(snapshot(mirror(&[2]))), r.now);

    assert_eq!(r.refused(Some("late"), None), [warning("late")]);

    assert_eq!(r.popup.mode(), Mode::List, "nothing to take back any more");
}

#[test]
fn a_refusal_does_not_open_the_grace_window_again_or_lose_it() {
    let mut r = Run::open(mirror(&[1, 2]));
    r.key("enter");
    r.key("enter");
    let timers = r.popup.timers();
    r.refused(None, None);
    assert_eq!(r.popup.timers(), timers);
}

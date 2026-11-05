//! The popup's deliberate deviations from Python that no captured scenario
//! reaches (tests/fixtures/README.md, "Deliberate deviations from Python").
//! The ones a scenario does reach replay from
//! tests/fixtures/popup-deviations.json in the transition and screen goldens.

mod popup;

use inbox_core::popup::Popup;
use popup::json;
use serde_json::{Value, json};

const DB: &str = "dddddddddddddddddddddddddddddddd";
const ROW: &str = "remote:m1|dddddddddddddddddddddddddddddddd|1";

fn rq(id: i64) -> Value {
    json!({"id": id, "group": 1, "status": "open", "pane_id": "w1:p1", "options": ["yes"],
           "multi_select": false, "has_preview": false, "db_id": DB, "qkey": qkey(id)})
}

fn qkey(id: i64) -> String {
    format!("{id:02}").repeat(8)
}

fn identity(id: i64, qkey: &str) -> Value {
    json!({"machine_id": "m1", "remote_id": id, "db_id": DB, "qkey": qkey})
}

/// A local question (so the popup never closes on the last row) and machine
/// m1's group 1 of `ids`, the mirror as `mirror` has it.
fn snapshot(label: &Value, ids: &[i64], mirror: &[Value]) -> Value {
    let local = json!({"id": 1, "group": 1, "status": "open", "pane_id": "w1:p2", "options": [],
                       "multi_select": false, "has_preview": false, "draft": null,
                       "delivery_error": null, "delivered": false});
    let questions: Vec<Value> = ids.iter().map(|id| rq(*id)).collect();
    let groups = if ids.is_empty() {
        json!([])
    } else {
        json!([{"key": format!("m1|{DB}|1"), "machine_id": "m1", "questions": questions}])
    };
    json!({"questions": [local], "open": [1], "failed": [], "alerts": [], "open_alerts": [],
           "remote": {"machines": [{"machine_id": "m1", "label": label}], "groups": groups,
                      "mirror": mirror, "alerts": [], "outbox": [], "problems": []}})
}

fn labelled(ids: &[i64]) -> Value {
    let mirror: Vec<Value> = ids.iter().map(|id| identity(*id, &qkey(*id))).collect();
    snapshot(&json!("project1"), ids, &mirror)
}

/// A popup the test drives, 600 ms a step (past every arming delay).
struct Rig {
    popup: Popup,
    at: u64,
}

impl Rig {
    fn open(snapshot: &Value) -> Self {
        Self::open_with(json!({"focus_pane": "w1:p2", "snapshot": snapshot}))
    }

    /// Held on the remote row, as a deep link opens it.
    fn held(snapshot: &Value) -> Self {
        let deeplink = json!({"target": ROW, "qid": 1, "hold": true, "missing": "gone"});
        Self::open_with(json!({"focus_pane": "w9:p9", "snapshot": snapshot, "deeplink": deeplink}))
    }

    fn open_with(open: Value) -> Self {
        let (popup, _) = Popup::open(json::open(&open), 10_000);
        Self { popup, at: 10_000 }
    }

    fn event(&mut self, event: &Value, after: u64) -> Vec<Value> {
        self.at += after;
        let effects = self.popup.handle(json::event(event), self.at);
        effects.iter().map(json::effect).collect()
    }

    fn key(&mut self, name: &str) -> Vec<Value> {
        self.event(&json!({"key": name}), 600)
    }

    fn state(&self) -> Value {
        json::state(&self.popup)
    }
}

fn notify(text: &str, severity: &str) -> Value {
    json!({"notify": {"text": text, "severity": severity}})
}

/// Quirk 1: nothing is focused on this machine, whatever the machine's name.
#[test]
fn chat_about_this_on_an_unlabelled_machine_names_it_and_stays() {
    let mirror = [identity(1, &qkey(1))];
    let mut rig = Rig::open(&snapshot(&Value::Null, &[1], &mirror));
    rig.key("down");
    rig.key("enter");
    rig.key("3"); // yes, Type something., Chat about this
    assert_eq!(rig.state()["view"]["cursor"], "chat");
    let text = "That agent runs on m1: open it from herdr's machine view";
    assert_eq!(rig.key("enter"), [notify(text, "information")]);
    assert_eq!(rig.state()["running"], true);
    assert_eq!(rig.state()["mode"], "remote");
    rig.key("escape");
    assert_eq!(rig.state()["mode"], "list");
}

/// Quirk 3: Python writes `None:Q1` here; a machine without a usable label
/// goes by its id.
#[test]
fn an_unlabelled_machines_closed_question_is_named_by_its_id() {
    let mirror = [identity(1, &qkey(1))];
    for label in [Value::Null, json!("")] {
        let mut rig = Rig::open(&snapshot(&label, &[1], &mirror));
        rig.key("down");
        rig.key("enter");
        rig.event(&json!({"poll": snapshot(&label, &[], &mirror)}), 100);
        let view = &rig.state()["view"];
        assert_eq!(view["kind"], "gone", "{label}");
        assert_eq!(view["ref"], "m1:Q1", "{label}");
    }
}

/// Quirk 4: the release is the group's: Tab or → once the popup has been up
/// for a moment, and the releasing key does nothing else.
#[test]
fn a_held_remote_question_is_released_only_by_a_deliberate_forward_key() {
    for release in ["tab", "right"] {
        let mut rig = Rig::held(&labelled(&[1]));
        assert_eq!(rig.state()["held"], true);
        rig.event(&json!({"key": release}), 100); // too soon after the popup appeared
        assert_eq!(rig.state()["held"], true, "{release} within the grace");
        for key in ["shift+tab", "left", "up", "down", "enter", "1"] {
            assert_eq!(rig.key(key), Vec::<Value>::new(), "{key}");
            assert_eq!(rig.state()["held"], true, "{key}");
            assert_eq!(rig.state()["focus"], "view", "{key} moves nothing");
        }
        assert_eq!(rig.key(release), Vec::<Value>::new());
        assert_eq!(rig.state()["held"], false, "{release}");
        assert_eq!(rig.state()["focus"], "view", "{release} does nothing else");
        assert_eq!(rig.state()["mode"], "remote");
        rig.key("tab");
        assert_eq!(rig.state()["focus"], "choices", "Tab is Textual's again");
    }
}

/// Quirk 5: Python's `LookupError` ends the popup here. The dismissals up to
/// that question are queued, the operator is told, and the row stays open.
#[test]
fn y_on_a_remote_group_stops_at_a_question_that_changed() {
    let first = identity(2, &qkey(2));
    let changed = identity(3, "ffffffffffffffff");
    for (mirror, why) in [
        (vec![first.clone()], "that remote question is gone"),
        (
            vec![first.clone(), changed],
            "that question changed — reopen it",
        ),
    ] {
        let mut rig = Rig::open(&labelled(&[2, 3]));
        rig.key("down");
        rig.key("d");
        assert!(!rig.state()["confirm"].is_null());
        let now = snapshot(&json!("project1"), &[2, 3], &mirror);
        rig.event(&json!({"data": now}), 100);
        assert_eq!(
            rig.key("y"),
            [
                json!({"remote_dismiss": first.clone()}),
                notify(&format!("not queued: {why}"), "warning"),
            ]
        );
        let state = rig.state();
        assert_eq!(state["running"], true);
        assert_eq!(state["closed"], json!({}));
        assert!(state["entries"].as_array().unwrap().contains(&json!(ROW)));
        // the row now holds what is left: the queued question is out of the model's copy
        rig.key("enter");
        assert_eq!(rig.state()["view"]["qid"], 3);
    }
}

//! The golden's JSON, both ways: snapshots and events in, the model's
//! state and effects out, in the golden's form (tests/fixtures/README.md,
//! "popup-transitions.json").

use inbox_core::popup::{
    Alert, DeepLink, Detail, Editing, Effect, Entry, Event, Key, Open, Popup, QOption, Question,
    ReviewCursor, Row, Send, Snapshot, View,
};
use serde_json::{Map, Value, json};

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn ids(v: &Value) -> Vec<i64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect()
}

pub fn detail(v: &Value) -> Option<Detail> {
    if v.is_null() {
        return None;
    }
    let choices = v["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| s(c).unwrap())
        .collect();
    let notes = v["notes"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, n)| (k.clone(), s(n).unwrap()))
        .collect();
    Some(Detail {
        choices,
        notes,
        text: s(&v["text"]),
    })
}

pub fn snapshot(v: &Value) -> Snapshot {
    let mut snap = Snapshot::default();
    for q in v["questions"].as_array().unwrap() {
        snap.questions.push(Question {
            id: q["id"].as_i64().unwrap(),
            group: q["group"].as_i64().unwrap(),
            status: s(&q["status"]).unwrap(),
            pane_id: s(&q["pane_id"]),
            options: q["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| QOption::label(o.as_str().unwrap()))
                .collect(),
            multi_select: q["multi_select"].as_bool().unwrap(),
            draft: detail(&q["draft"]),
            delivery_error: s(&q["delivery_error"]),
            delivered: q["delivered"].as_bool().unwrap(),
            ..Question::default()
        });
    }
    snap.open = ids(&v["open"]);
    snap.failed = ids(&v["failed"]);
    for a in v["alerts"].as_array().unwrap() {
        snap.alerts.push(Alert {
            id: a["id"].as_i64().unwrap(),
            kind: s(&a["kind"]).unwrap(),
            status: s(&a["status"]).unwrap(),
            pane_id: s(&a["pane_id"]),
            ..Alert::default()
        });
    }
    snap.open_alerts = ids(&v["open_alerts"]);
    snap
}

pub fn open(v: &Value) -> Open {
    let deeplink = v.get("deeplink").map(|d| DeepLink {
        target: d["target"].as_str().map(|t| Entry::parse(t).unwrap()),
        qid: d["qid"].as_i64(),
        hold: d["hold"].as_bool().unwrap(),
        missing: s(&d["missing"]).unwrap(),
    });
    Open {
        focus_pane: s(&v["focus_pane"]),
        snapshot: snapshot(&v["snapshot"]),
        deeplink,
    }
}

pub fn event(v: &Value) -> Event {
    if let Some(k) = v.get("key") {
        return Event::Key(Key::parse(k.as_str().unwrap()));
    }
    if let Some(p) = v.get("poll") {
        return Event::Poll(snapshot(p));
    }
    if let Some(d) = v.get("sent") {
        return Event::Sent {
            done: ids(&d["done"]),
            wanted: d["wanted"].as_u64().unwrap() as usize,
            snapshot: snapshot(&d["snapshot"]),
        };
    }
    if let Some(f) = v.get("focused") {
        return Event::Focused {
            pane: s(&f["pane"]).unwrap(),
            error: s(&f["error"]),
        };
    }
    panic!("unknown event {v}")
}

fn detail_json(d: &Option<Detail>) -> Value {
    match d {
        None => Value::Null,
        Some(d) => {
            let notes: Map<String, Value> =
                d.notes.iter().map(|(k, n)| (k.clone(), json!(n))).collect();
            json!({"choices": d.choices, "notes": notes, "text": d.text})
        }
    }
}

pub fn effect(e: &Effect) -> Value {
    match e {
        Effect::Notify { text, severity } => {
            json!({"notify": {"text": text, "severity": severity.name()}})
        }
        Effect::Draft { qid, detail } => {
            json!({"draft": {"qid": qid, "detail": detail_json(detail)}})
        }
        Effect::Send(Send::Answer { qid, detail }) => {
            json!({"send": {"kind": "answer", "qid": qid, "detail": detail_json(detail)}})
        }
        Effect::Send(Send::Submit { qids }) => json!({"send": {"kind": "submit", "qids": qids}}),
        Effect::Send(Send::Retry { qid }) => json!({"send": {"kind": "retry", "qid": qid}}),
        Effect::Dismiss { qid } => json!({"dismiss": {"qid": qid}}),
        Effect::Hide { qid } => json!({"hide": {"qid": qid}}),
        Effect::Ack { alert } => json!({"ack": {"alert": alert}}),
        Effect::GoTo { pane, graced } => json!({"go_to": {"pane": pane, "graced": graced}}),
        Effect::Exit => json!({"exit": {}}),
        Effect::Quit => json!({"quit": {}}),
        other => panic!("no golden form for {other:?}"),
    }
}

fn view(p: &Popup) -> Value {
    match p.view() {
        View::None => json!({"kind": "none"}),
        View::List(v) => {
            json!({"kind": "list", "cursor": v.cursor.as_ref().map(Entry::to_string), "armed_at": v.armed_at})
        }
        View::Question(v) => {
            let cursor = match v.rows[v.cursor] {
                Row::Option(i) => format!("opt:{i}"),
                Row::Other => "other".into(),
                Row::Chat => "chat".into(),
                Row::Submit => "submit".into(),
            };
            let notes: Map<String, Value> = v
                .notes
                .iter()
                .map(|(i, n)| (i.to_string(), json!(n)))
                .collect();
            let (editing, input) = match &v.editing {
                None => (Value::Null, Value::Null),
                Some(e) => {
                    let name = match e {
                        Editing::Note(i, _) => format!("note:{i}"),
                        Editing::Text(_) => "text".into(),
                    };
                    let b = e.text_box();
                    (
                        json!(name),
                        json!({"value": b.value(), "cursor": b.cursor()}),
                    )
                }
            };
            json!({"kind": "question", "qid": v.qid, "in_group": v.in_group, "last": v.last, "cursor": cursor,
                   "picked": v.picked.iter().collect::<Vec<_>>(), "notes": notes, "text": v.text,
                   "editing": editing, "input": input, "armed_at": v.armed_at})
        }
        View::Review(v) => {
            let cursor = match v.cursor {
                None => Value::Null,
                Some(ReviewCursor::Item(id)) => json!(format!("item:{id}")),
                Some(ReviewCursor::Submit) => json!("submit"),
                Some(ReviewCursor::Cancel) => json!("cancel"),
            };
            json!({"kind": "review", "items": v.items, "cursor": cursor, "armed_at": v.armed_at})
        }
        View::Gone(v) => json!({"kind": "gone", "ref": v.r#ref, "reason": v.reason}),
        View::Failed(v) => {
            json!({"kind": "failed", "qid": v.qid, "cursor": v.cursor, "armed_at": v.armed_at})
        }
        View::Alert(v) => {
            json!({"kind": "alert", "alert": v.alert, "label": null, "armed_at": v.armed_at})
        }
    }
}

/// The model's state in the golden's form.
pub fn state(p: &Popup) -> Value {
    if !p.running() {
        return json!({"running": false});
    }
    let closed: Map<String, Value> = p
        .closed()
        .iter()
        .map(|(e, r)| (e.to_string(), json!(r)))
        .collect();
    let fresh: Map<String, Value> = p
        .entries()
        .iter()
        .filter_map(|e| {
            p.arrived_at(e)
                .map(|t| (e.to_string(), json!(t + inbox_core::popup::ROW_ARM)))
        })
        .collect();
    let gone: Map<String, Value> = p
        .gone()
        .iter()
        .map(|(t, r)| (t.to_string(), json!(r)))
        .collect();
    let t = p.timers();
    json!({
        "running": true,
        "mode": p.mode().name(),
        "entries": p.entries().iter().map(Entry::to_string).collect::<Vec<_>>(),
        "closed": closed,
        "fresh_until": fresh,
        "list_cursor": p.list_cursor().map(Entry::to_string),
        "direct": p.direct(),
        "held": p.held(),
        "entry": p.entry().map(Entry::to_string),
        "tabs": p.tabs().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "current": p.current().map(|t| t.to_string()),
        "gone": gone,
        "single": p.single(),
        "grace_until": t.grace_until,
        "view_at": t.view_at,
        "last_key_at": t.last_key_at,
        "prev_key_at": t.prev_key_at,
        "view": view(p),
        "confirm": p.confirm().map(|c| c.what.clone()),
    })
}

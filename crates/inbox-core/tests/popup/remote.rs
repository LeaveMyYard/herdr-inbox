//! The golden's remote-machine snapshot part and identities, both ways
//! (tests/fixtures/README.md, "popup-transitions.json", remote machines).

use inbox_core::popup::{
    Machine, OutboxRow, Problem, Remote, RemoteAlert, RemoteGroup, RemoteQuestion, RemoteRef,
};
use serde_json::{Value, json};

fn s(v: &Value) -> String {
    v.as_str().unwrap().to_string()
}

fn each(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}

pub fn remote(v: &Value) -> Remote {
    let mut r = Remote::default();
    for m in each(v, "machines") {
        r.machines.push(Machine {
            machine_id: s(&m["machine_id"]),
            label: m["label"].as_str().map(str::to_string),
            offline: None,
        });
    }
    for g in each(v, "groups") {
        let machine_id = s(&g["machine_id"]);
        let questions = each(&g, "questions")
            .iter()
            .map(|q| RemoteQuestion {
                machine_id: machine_id.clone(),
                db_id: s(&q["db_id"]),
                qkey: s(&q["qkey"]),
                question: super::json::question(q),
            })
            .collect();
        r.groups.push(RemoteGroup {
            key: s(&g["key"]),
            machine_id,
            questions,
        });
    }
    for m in each(v, "mirror") {
        r.mirror.push(RemoteRef {
            machine_id: s(&m["machine_id"]),
            remote_id: m["remote_id"].as_i64().unwrap(),
            db_id: s(&m["db_id"]),
            qkey: s(&m["qkey"]),
        });
    }
    for a in each(v, "alerts") {
        r.alerts.push(RemoteAlert {
            machine_id: s(&a["machine_id"]),
            db_id: s(&a["db_id"]),
            akey: s(&a["akey"]),
            alert: super::json::alert(&a),
        });
    }
    for o in each(v, "outbox") {
        r.outbox.push(OutboxRow {
            request_id: s(&o["request_id"]),
            machine_id: s(&o["machine_id"]),
            action: s(&o["action"]),
            remote_id: o["remote_id"].as_i64().unwrap(),
            state: s(&o["state"]),
            clearable: o["clearable"].as_bool().unwrap(),
            ..OutboxRow::default()
        });
    }
    for p in each(v, "problems") {
        r.problems.push(Problem {
            machine_id: s(&p["machine_id"]),
            remote_id: p["remote_id"].as_i64().unwrap(),
            ..Problem::default()
        });
    }
    r
}

pub fn ref_json(r: &RemoteRef) -> Value {
    json!({"machine_id": r.machine_id, "remote_id": r.remote_id, "db_id": r.db_id, "qkey": r.qkey})
}

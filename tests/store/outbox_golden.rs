//! The outbox replays tests/fixtures/store-outbox.json: Python's `remote.py`
//! over `STORE_OUTBOX_OPS` (enqueueing and its refusals, flushes with every
//! reply the remote can give, retries and backoff, re-validation against the
//! mirror, expiry and the status query, a flush stopped by Python raising,
//! an unsafe target, the sweep, the popup's calls), ssh scripted at
//! `subprocess.run`, with the outbox and the mirrored alerts after each step,
//! every ssh call (and the row's `sending_at` while an apply runs) and every
//! notification (compared as two sequences: Python sends a notice mid-flush,
//! the port hands them back when the flush is done).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_inbox::pyjson;
use herdr_inbox::remote::command::{FixedCommand, PROBE, SafeTarget};
use herdr_inbox::store::Store;
use herdr_inbox::store::mirror::{CallResult, Notice, OutboxRow};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};

use crate::common::migrated;
use crate::golden::{s, sql};

const GOLDEN: &str = include_str!("../fixtures/store-outbox.json");
const PROBE_OUT: &str = "/home/u/.local/bin/uv\n/usr/local/bin/herdr\nmissing:claude\n";
const EXPIRY: Duration = Duration::from_secs(12 * 3600);

fn plugins_out() -> String {
    pyjson::dumps(&json!({"result": {"plugins": [
        {"plugin_id": "other.plugin", "plugin_root": "/x"},
        {"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "/home/u/.config/herdr/plugins/inbox"}]}}))
}

fn tables(store: &Store) -> Value {
    let mut out = Map::new();
    for table in ["remote_outbox", "remote_alerts"] {
        let mut stmt = store
            .db()
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Value> = stmt
            .query_map([], |r| {
                let mut row = Map::new();
                for (i, name) in names.iter().enumerate() {
                    row.insert(
                        name.clone(),
                        match r.get_ref(i)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => json!(n),
                            ValueRef::Real(f) => json!(f),
                            ValueRef::Text(t) => json!(std::str::from_utf8(t).unwrap()),
                            ValueRef::Blob(_) => panic!("blob"),
                        },
                    );
                }
                Ok(Value::Object(row))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        out.insert(table.into(), Value::Array(rows));
    }
    Value::Object(out)
}

fn row(r: &OutboxRow) -> Value {
    json!({"request_id": r.request_id, "machine_id": r.machine_id, "db_id": r.db_id, "remote_id": r.remote_id,
           "qkey": r.qkey, "action": r.action, "detail_json": r.detail_json, "created_at": r.created_at,
           "expires_at": r.expires_at, "state": r.state, "result": r.result, "sent_at": r.sent_at,
           "attempts": r.attempts, "next_try_at": r.next_try_at, "sending_at": r.sending_at, "label": r.label})
}

fn notices(effects: &mut Vec<Value>, list: Vec<Notice>) {
    effects.extend(list.into_iter().map(|n| json!(["notify", n.title, n.body])));
}

struct Replay {
    store: Store,
    now: u64,
    uuids: u64,
}

impl Replay {
    fn at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.now)
    }

    fn run(&mut self, op: &str, a: &Value) -> (Value, Vec<Value>) {
        if let Some(n) = a.get("now").and_then(Value::as_u64) {
            self.now = n;
        }
        let now = self.at();
        let mut effects: Vec<Value> = Vec::new();
        let mut replies: Vec<Value> = a
            .get("replies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        replies.reverse();
        let store = &self.store;
        let result = {
            let effects_cell = std::cell::RefCell::new(&mut effects);
            let mut ssh = |target: &SafeTarget, command: &FixedCommand, stdin: Option<&str>| {
                let (target, command) = (target.as_str(), command.as_str());
                let mut fx = effects_cell.borrow_mut();
                match stdin {
                    None => {
                        fx.push(json!(["ssh", target, command]));
                        let out = if command == PROBE {
                            a.get("probe")
                                .and_then(Value::as_str)
                                .unwrap_or(PROBE_OUT)
                                .to_string()
                        } else {
                            a.get("plugins")
                                .and_then(Value::as_str)
                                .map_or_else(plugins_out, String::from)
                        };
                        CallResult {
                            code: 0,
                            stdout: out,
                            stderr: String::new(),
                        }
                    }
                    Some(input) if command.ends_with(" inbox export") => {
                        fx.push(json!(["call", target, command, input]));
                        CallResult {
                            code: a.get("code").and_then(Value::as_i64).unwrap_or(0) as i32,
                            stdout: a.get("export").map_or_else(String::new, pyjson::dumps),
                            stderr: String::new(),
                        }
                    }
                    Some(input) => {
                        let rid = serde_json::from_str::<Value>(input).unwrap()["request_id"]
                            .as_str()
                            .unwrap()
                            .to_string();
                        let sending: Option<String> = store
                            .db()
                            .query_row(
                                "SELECT sending_at FROM remote_outbox WHERE request_id = ?",
                                [&rid],
                                |r| r.get(0),
                            )
                            .ok()
                            .flatten();
                        fx.push(json!(["sending", rid, sending]));
                        fx.push(json!(["call", target, command, input]));
                        let r = replies
                            .pop()
                            .unwrap_or_else(|| json!({"code": 255, "stderr": "ssh: no reply"}));
                        if let Some(before) = r.get("before").and_then(Value::as_str) {
                            // another worker settles the row while this send is under way
                            store.db().execute_batch(before).unwrap();
                        }
                        CallResult {
                            code: r.get("code").and_then(Value::as_i64).unwrap_or(0) as i32,
                            stdout: r.get("reply").map_or_else(
                                || s(&r, "stdout").unwrap_or("").to_string(),
                                pyjson::dumps,
                            ),
                            stderr: s(&r, "stderr").unwrap_or("").into(),
                        }
                    }
                }
            };
            match op {
                "sql" => {
                    sql(store, a);
                    Value::Null
                }
                "pull" => {
                    let m = &a["machine"];
                    let (ok, got) = store
                        .pull(
                            m["id"].as_str().unwrap(),
                            m["label"].as_str(),
                            m["target"].as_str(),
                            now,
                            &mut ssh,
                        )
                        .unwrap();
                    notices(&mut effects_cell.borrow_mut(), got);
                    json!(ok)
                }
                "enqueue" | "enqueue_alert_ack" => {
                    let data = a["data"].as_object().unwrap();
                    let rid = format!("{:032x}", self.uuids + 1);
                    let got = if op == "enqueue" {
                        self.uuids += 1; // Python takes the uuid before it looks
                        store
                            .enqueue(
                                s(a, "machine_id").unwrap(),
                                data,
                                s(a, "action").unwrap(),
                                a.get("detail").filter(|d| !d.is_null()),
                                &rid,
                                now,
                                EXPIRY,
                            )
                            .unwrap()
                    } else {
                        let got = store
                            .enqueue_alert_ack(s(a, "machine_id").unwrap(), data, &rid, now, EXPIRY)
                            .unwrap();
                        if got.is_ok() {
                            self.uuids += 1; // only after the lookup succeeded
                        }
                        got
                    };
                    match got {
                        Ok(id) => json!(id),
                        Err(e) => json!({"error": "LookupError", "message": e.0}),
                    }
                }
                "flush" => {
                    let got = store
                        .flush(s(a, "machine_id").unwrap(), now, &mut ssh)
                        .unwrap();
                    notices(&mut effects_cell.borrow_mut(), got.notices);
                    match got.aborted {
                        None => Value::Null,
                        // Python's json.loads message is not serde's: the kind is compared, the
                        // text is documented as differing
                        Some(m) if m.starts_with(herdr_inbox::store::mirror::CORRUPT_DETAIL) => {
                            json!({"error": "JSONDecodeError"})
                        }
                        Some(m) => {
                            json!({"error": if m.contains("has no attribute") { "AttributeError" } else { "ValueError" }, "message": m})
                        }
                    }
                }
                "expire_sweep" => {
                    notices(
                        &mut effects_cell.borrow_mut(),
                        store.expire_sweep(now).unwrap(),
                    );
                    Value::Null
                }
                "clearable" => {
                    json!(store.clearable(s(a, "request_id").unwrap(), now).unwrap())
                }
                "mark_seen" => {
                    store.mark_seen(s(a, "request_id").unwrap()).unwrap();
                    Value::Null
                }
                "outbox_rows" => {
                    Value::Array(store.outbox_rows().unwrap().iter().map(row).collect())
                }
                other => panic!("no Rust call for {other}"),
            }
        };
        (result, effects)
    }
}

#[test]
fn every_outbox_step_leaves_pythons_rows_calls_and_notices() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    let mut replay = Replay {
        store,
        now: 0,
        uuids: 0,
    };
    let steps = golden["steps"].as_array().unwrap();
    assert!(steps.len() >= 50);
    for (n, step) in steps.iter().enumerate() {
        let op = step["op"].as_str().unwrap();
        let (result, effects) = replay.run(op, &step["args"]);
        let mut want = step["result"].clone();
        if want.get("error") == Some(&json!("JSONDecodeError")) {
            want = json!({"error": "JSONDecodeError"});
        }
        assert_eq!(result, want, "step {n} {op}: result");
        // Python notifies in the middle of a flush; the port returns its notices for the caller
        // to send afterwards. Each sequence is Python's: the ssh calls, and the notices.
        let split = |list: &[Value]| -> (Vec<Value>, Vec<Value>) {
            list.iter().cloned().partition(|e| e[0] != json!("notify"))
        };
        let (got_calls, got_notices) = split(&effects);
        let (want_calls, want_notices) = split(step["effects"].as_array().unwrap());
        assert_eq!(got_calls, want_calls, "step {n} {op}: ssh calls");
        assert_eq!(got_notices, want_notices, "step {n} {op}: notices");
        assert_eq!(
            tables(&replay.store),
            step["tables"],
            "step {n} {op}: tables"
        );
    }
}

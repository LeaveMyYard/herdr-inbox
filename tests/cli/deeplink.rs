//! The deep-link steps of tests/fixtures/cli.json: the request `inbox open`
//! leaves, and the popup's side (Python `tui.on_mount`): the request taken
//! with the popup's nonce, then resolved.

use std::path::Path;
use std::time::Duration;

use herdr_inbox::deeplink::request::{consume_request, request_path, unix_now};
use herdr_inbox::deeplink::resolve::{MachineName, Target, resolve};
use herdr_inbox::deeplink::{Request, describe};
use herdr_inbox::store::Store;
use serde_json::{Map, Value, json};

use crate::fakes::Effects;
use crate::store_golden::s;

/// The request as Python's `consume_request` returns it (`hold` only when false).
fn request_json(r: &Request) -> Value {
    let mut out = json!({"kind": r.kind, "value": r.value});
    if !r.hold {
        out["hold"] = json!(false);
    }
    out
}

/// What `inbox open` left for the popup, masked as the capture masks it, or
/// `None`; the file is removed, as the capture removes it.
pub fn left_request(state_dir: &Path) -> Value {
    let path = request_path(state_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Value::Null;
    };
    std::fs::remove_file(&path).unwrap();
    let data: Map<String, Value> = serde_json::from_str(&text).unwrap();
    let masked = data.into_iter().map(|(k, v)| match k.as_str() {
        "nonce" | "created_at" => (k.clone(), json!(format!("<{k}>"))),
        _ => (k, v),
    });
    Value::Object(masked.collect())
}

/// A `deeplink` step: the request file written, taken and resolved.
pub fn step(db: &Path, state_dir: &Path, a: &Value, effects: &Effects) -> Value {
    let path = request_path(state_dir);
    let _ = std::fs::remove_file(&path);
    if let Some(raw) = s(a, "raw") {
        std::fs::write(&path, raw).unwrap();
    } else if let Some(Value::Object(req)) = a.get("request") {
        let mut data = req.clone();
        let age = a.get("age").and_then(Value::as_f64).unwrap_or(0.0);
        data.insert("created_at".into(), json!(unix_now() - age));
        data.insert("nonce".into(), json!("n1"));
        std::fs::write(&path, herdr_inbox::pyjson::dumps(&Value::Object(data))).unwrap();
    }
    let request = consume_request(state_dir, s(a, "nonce"), unix_now());
    let mut out = json!({"request": request.as_ref().map(request_json), "left": path.exists()});
    let _ = std::fs::remove_file(&path);
    let Some(request) = request else {
        return out;
    };
    let store = Store::open(db, Duration::from_secs(5)).unwrap();
    let machines = || {
        effects.borrow_mut().push(json!(["machines"]));
        a.get("machines")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|m| MachineName {
                        label: s(m, "label").map(str::to_string),
                        id: s(m, "id").map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let resolved = resolve(&store, &request, machines).unwrap();
    out["resolved"] = match resolved {
        None => Value::Null,
        Some((Target::Group(g), qid)) => json!([["group", g], qid]),
        Some((Target::Remote(key), qid)) => json!([["remote", key], qid]),
    };
    out["describe"] = json!(describe(&request.kind, &request.value));
    out
}

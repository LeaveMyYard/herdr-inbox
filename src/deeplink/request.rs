//! The one-shot focus request (Python `deeplink.write_request`,
//! `consume_request`): `<state>/open-request.json`, written atomically
//! (temp file, then rename over it: the newest request wins) and taken by an
//! atomic rename (two popups starting at once cannot both take it), then
//! deleted, and honoured only if it is the taker's and fresh.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::{KINDS, Request, valid};
use crate::pyjson::dumps;
use crate::token::hex8;

/// How old a request may be when the popup takes it (Python `REQUEST_TTL`).
pub const REQUEST_TTL: Duration = Duration::from_secs(5);

/// The request file in the inbox's state directory.
pub fn request_path(state_dir: &Path) -> PathBuf {
    state_dir.join("open-request.json")
}

/// Unix seconds now, as Python's `time.time()`.
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// Writes a request for `kind` `value`, made `now`; returns its nonce (the
/// popup gets it in [`super::NONCE_ENV`]). `None` when the value is not a
/// valid `kind` (Python raises) or the file cannot be written.
pub fn write_request(
    state_dir: &Path,
    kind: &str,
    value: &str,
    hold: bool,
    now: f64,
) -> Option<String> {
    if !valid(kind, value) {
        return None;
    }
    fs::create_dir_all(state_dir).ok()?;
    let nonce: String = (0..4).map(|_| hex8()).collect();
    let mut data = json!({"kind": kind, "value": value, "created_at": now, "nonce": nonce});
    if !hold {
        data["hold"] = json!(false);
    }
    let tmp = state_dir.join(format!(
        ".open-request-{}-{}",
        std::process::id(),
        &nonce[..8]
    ));
    fs::write(&tmp, dumps(&data)).ok()?;
    if fs::rename(&tmp, request_path(state_dir)).is_err() {
        let _ = fs::remove_file(&tmp);
        return None;
    }
    Some(nonce)
}

/// Takes the pending request if this popup was opened for it (`nonce`, its
/// [`super::NONCE_ENV`]), once, whoever asks first. `None` when there is
/// none, it is another's (left in place), or it is stale, unreadable or
/// malformed (taken and dropped). A popup opened by hand (no nonce) never
/// takes one.
pub fn consume_request(state_dir: &Path, nonce: Option<&str>, now: f64) -> Option<Request> {
    let nonce = nonce.filter(|n| !n.is_empty())?;
    let path = request_path(state_dir);
    if !path.exists() {
        return None;
    }
    if let Ok(text) = fs::read_to_string(&path)
        && let Ok(data) = serde_json::from_str::<Value>(&text)
    {
        // a readable request that is not ours stays for its popup (Python reads only a dict's)
        if data.as_object()?.get("nonce").and_then(Value::as_str) != Some(nonce) {
            return None;
        }
    }
    let taken = state_dir.join(format!(
        ".open-request-taken-{}-{}",
        std::process::id(),
        hex8()
    ));
    fs::rename(&path, &taken).ok()?;
    let data = fs::read_to_string(&taken)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let _ = fs::remove_file(&taken);
    let data = data?;
    let data = data.as_object()?;
    let kind = data
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| KINDS.contains(k))?;
    if data.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return None;
    }
    let age = now - created_at(data)?;
    if !(0.0..=REQUEST_TTL.as_secs_f64()).contains(&age) {
        return None;
    }
    Some(Request {
        kind: kind.to_string(),
        value: data.get("value").map_or_else(String::new, py_str),
        hold: data.get("hold") != Some(&Value::Bool(false)),
    })
}

/// Python's `float(data.get("created_at") or 0)`; `None` where it raises.
fn created_at(data: &Map<String, Value>) -> Option<f64> {
    match data.get("created_at") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Some(0.0),
        Some(Value::Bool(true)) => Some(1.0),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if s.is_empty() => Some(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        Some(_) => None,
    }
}

/// Python's `str()` of a JSON value, for the scalars a request holds.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;

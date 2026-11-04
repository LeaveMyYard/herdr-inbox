//! `inbox export` and `inbox apply` (Python `cli.main`'s branch,
//! `remote_api.run_cli`): the request is JSON on stdin, the reply one JSON
//! object on stdout as Python's `json.dumps` writes it. They never migrate:
//! on a database without this code's schema they start this machine's
//! worker, which does, and say `not ready`. **SAFETY**: `apply` writes the
//! operator's answers, dismissals and acknowledgements (see
//! [`crate::remote_api::apply`]).

use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::answer::with_markers;
use super::{Io, failed};
use crate::pyjson::{dumps, loads, lossless};
use crate::remote_api::apply::{After, apply};
use crate::remote_api::export::export;
use crate::store::readiness::{expected, missing_from};
use crate::store::revision::MIGRATION_REVISION;
use crate::store::revision_wanted::{unix_now, want};
use crate::store::{OpenError, Store};

/// How long a statement waits for the write lock (Python `Store(timeout=5)`).
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `export` or `apply` on the request read from `io.input`; the exit code.
pub fn run(cmd: &str, io: &mut Io<'_>) -> i32 {
    let mut raw = Vec::new();
    let read = io.input.read_to_end(&mut raw);
    let request = match read.map_err(|e| e.to_string()).and_then(|_| parse(raw)) {
        Ok(r) => r,
        Err(why) => return reply(io, 1, &json!({"error": format!("bad request: {why}")})),
    };
    let store = match Store::open(io.db_path, TIMEOUT) {
        Ok(store) => store,
        Err(OpenError::NotReady(missing)) => return not_ready(io, missing),
        Err(OpenError::Sqlite(_)) if !io.db_path.exists() => {
            // Python's Store() creates the file and finds it lacks everything; this leaves
            // creating it to the worker, which migrates it
            want(io.db_path, MIGRATION_REVISION, unix_now());
            return not_ready(io, lacking_everything());
        }
        Err(OpenError::Sqlite(err)) => return failed(io, &err),
    };
    let done = match cmd {
        "export" => export(&store, &request.map, SystemTime::now()).map(|out| (out, None)),
        // a lone surrogate reads as U+FFFD here: never record or write that (Python's
        // SQLite write raises on it); a request carrying one in what apply writes is
        // refused, unrecorded
        _ if !request.lossless && written_lossily(&request.map) => Ok((
            json!({"outcome": "rejected", "reason": "bad request"}),
            None,
        )),
        _ => apply(&store, &request.map),
    };
    match done {
        Ok((out, after)) => {
            if let Some(after) = after {
                run_after(&store, after, io);
            }
            reply(io, 0, &out)
        }
        Err(err) => failed(io, &err),
    }
}

/// A request read from stdin.
struct Request {
    map: Map<String, Value>,
    /// It reads without loss ([`lossless`]).
    lossless: bool,
}

/// The request (Python `json.loads`, then "not an object" for anything but
/// one). Bytes that are not UTF-8 are a bad request here (Python's stdin
/// read raises).
fn parse(raw: Vec<u8>) -> Result<Request, String> {
    let text = String::from_utf8(raw).map_err(|e| e.to_string())?;
    match loads(&text).map_err(|e| e.to_string())? {
        Value::Object(map) => Ok(Request {
            map,
            lossless: lossless(&text),
        }),
        _ => Err("not an object".into()),
    }
}

/// Whether a field `apply` records or writes (`request_id`, `id`, `detail`) may
/// hold a lone surrogate read as U+FFFD. Called only for a request whose text
/// is not lossless: a U+FFFD there is taken for one (a real U+FFFD in the same
/// request as a surrogate elsewhere is refused too, safely). A surrogate only
/// in a field apply compares (`db_id`, `qkey`: hex, so U+FFFD never matches)
/// or ignores does not refuse it, as Python applies it.
fn written_lossily(map: &Map<String, Value>) -> bool {
    fn has(v: &Value) -> bool {
        match v {
            Value::String(s) => s.contains('\u{fffd}'),
            Value::Array(items) => items.iter().any(has),
            Value::Object(m) => m.iter().any(|(k, v)| k.contains('\u{fffd}') || has(v)),
            _ => false,
        }
    }
    ["request_id", "id", "detail"]
        .iter()
        .any(|k| map.get(*k).is_some_and(has))
}

/// What a committed `apply` does next: the detached delivery, then the
/// pane's marks (Python `apply`'s `after`).
fn run_after(store: &Store, after: After, io: &mut Io<'_>) {
    let pane = match after {
        After::Markers(pane) => pane,
        After::Deliver { qid, force, pane } => {
            io.delivery.spawn_deliver(&[qid], force);
            pane
        }
    };
    with_markers(store, io, |m, _| m.update(pane.as_deref()));
}

/// The schema is not there: start the worker, which migrates it, and say so.
fn not_ready(io: &mut Io<'_>, missing: Vec<String>) -> i32 {
    io.delivery.ensure_worker(None); // the next pull gets through
    let why = OpenError::NotReady(missing);
    reply(io, 1, &json!({"error": format!("not ready: {why}")}))
}

/// What a database without any schema lacks (Python's freshly created file).
fn lacking_everything() -> Vec<String> {
    Connection::open_in_memory()
        .and_then(|db| missing_from(&db, expected()))
        .unwrap_or_default()
}

/// Prints `out` (Python `print(json.dumps(out))`); returns `code`.
fn reply(io: &mut Io<'_>, code: i32, out: &Value) -> i32 {
    let _ = writeln!(io.out, "{}", dumps(out));
    code
}

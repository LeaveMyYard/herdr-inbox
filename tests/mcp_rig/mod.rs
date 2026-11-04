//! Replays a recorded MCP session against `herdr-inbox mcp`: the recording's
//! client lines go to the binary one at a time, each request waits for its
//! reply. The binary runs with its environment cleared (no HERDR_* variable,
//! no worker) and its database and state in a temp dir.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

/// A recording under `tests/fixtures/mcp`.
pub fn golden(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mcp")
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The messages of one direction.
pub fn of(recs: &[Value], dir: &str) -> Vec<Value> {
    recs.iter()
        .filter(|r| r["dir"] == dir)
        .map(|r| r["msg"].clone())
        .collect()
}

/// The reply to request `id`.
pub fn by_id<'a>(msgs: &'a [Value], id: &Value) -> &'a Value {
    msgs.iter()
        .find(|m| m.get("id") == Some(id) && m.get("method").is_none())
        .unwrap_or_else(|| panic!("no reply {id}"))
}

/// The binary's replies to `requests`, and the temp dir it ran in.
pub fn replay(requests: &[Value]) -> (Vec<Value>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-inbox"))
        .env_clear()
        .arg("mcp")
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.path())
        .env("XDG_STATE_HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env("HERDR_INBOX_DB", dir.path().join("inbox.db"))
        .env("HERDR_INBOX_NO_WORKER", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(serde_json::from_str::<Value>(&line).unwrap());
        }
    });
    let mut stdin = child.stdin.take().unwrap();
    let mut got = Vec::new();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
        stdin.flush().unwrap();
        let Some(id) = r.get("id") else { continue };
        let listen = r["method"] == "subscriptions/listen";
        loop {
            let m = rx.recv_timeout(Duration::from_secs(20)).expect("a reply");
            let done = (listen && m["method"] == "notifications/subscriptions/acknowledged")
                || m.get("id") == Some(id);
            got.push(m);
            if done {
                break;
            }
        }
    }
    drop(stdin);
    let _ = child.wait();
    (got, dir)
}

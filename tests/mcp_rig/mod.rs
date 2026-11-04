//! Replays a recorded MCP session against `herdr-inbox mcp`: the recording's
//! client lines go to the binary one at a time, each request waits for its
//! reply. The binary runs with its environment cleared (no HERDR_* variable but
//! `HERDR_BIN_PATH`, a logging stub; no worker) and its database and state in a
//! temp dir.

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

/// The binary's replies to `requests` on a migrated database, and the temp
/// dir it ran in.
pub fn replay(requests: &[Value]) -> (Vec<Value>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    // as the capture leaves it: migrated, as the worker leaves it (tools never migrate)
    let db = dir.path().join("inbox.db");
    herdr_inbox::store::Store::open_migrating(&db, std::time::Duration::from_secs(3)).unwrap();
    let recs: Vec<Value> = requests
        .iter()
        .map(|r| serde_json::json!({"dir": "client->server", "msg": r}))
        .collect();
    let got = session(&recs, dir.path(), &[], &mut |_, _| {});
    (got, dir)
}

/// The binary's replies to a recorded session, run in `dir` (its database
/// `dir/inbox.db`, migrated or not as the caller left it) with `extra`
/// variables; each `"dir": "action"` record is handed to `act` between two
/// requests.
pub fn session(
    recs: &[Value],
    dir: &Path,
    extra: &[(&str, String)],
    act: &mut dyn FnMut(&Value, &Path),
) -> Vec<Value> {
    // herdr's CLI is a stub that only logs (never the machine's own herdr, which `PATH`
    // could find): every CLI call the server makes lands in `herdr-cli.log`
    let stub = dir.join("herdr-cli");
    std::fs::write(&stub, "#!/bin/sh\necho \"$@\" >> \"$0.log\"\nexit 1\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-inbox"));
    cmd.env_clear()
        .arg("mcp")
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .env("XDG_STATE_HOME", dir)
        .env("XDG_CONFIG_HOME", dir)
        .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
        .env("HERDR_INBOX_DB", dir.join("inbox.db"))
        .env("HERDR_INBOX_NO_WORKER", "1")
        .env("HERDR_BIN_PATH", &stub)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    let (tx, rx) = mpsc::channel();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(serde_json::from_str::<Value>(&line).unwrap());
        }
    });
    let mut stdin = child.stdin.take().unwrap();
    let mut got = Vec::new();
    for rec in recs {
        if rec["dir"] == "action" {
            act(&rec["msg"], dir);
            continue;
        }
        if rec["dir"] != "client->server" {
            continue;
        }
        let r = &rec["msg"];
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
    got
}

/// The herdr CLI calls the server made in `dir` (through the stub), one per line.
pub fn cli_calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("herdr-cli.log"))
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

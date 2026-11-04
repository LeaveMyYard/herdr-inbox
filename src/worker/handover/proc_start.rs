//! A process's start time, the token that tells a pid's process from a
//! later one that reused the pid (Python `worker_handover.start_time`).

use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The longest `ps` may take.
const PS_TIMEOUT: Duration = Duration::from_secs(5);

/// When process `pid` started, as an opaque token, `-` when it cannot be
/// read: Linux `/proc/<pid>/stat` field 22, elsewhere the words of
/// `ps -o lstart= -p <pid>` joined with `_`. Never fails.
pub fn start_time(pid: u32) -> String {
    proc_stat(pid)
        .or_else(|| ps_lstart(pid))
        .unwrap_or_else(|| "-".to_string())
}

/// Field 22 of `/proc/<pid>/stat` (the 20th after the `)` closing the name).
fn proc_stat(pid: u32) -> Option<String> {
    let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.iter().rposition(|b| *b == b')')?;
    let rest = std::str::from_utf8(stat.get(close + 2..)?).ok()?;
    rest.split_whitespace().nth(19).map(String::from)
}

/// `ps -o lstart=`, given at most [`PS_TIMEOUT`] (Python's `timeout=5`): a
/// hung `ps` is killed (it is this process's own unreaped child) and reads
/// as unknown.
fn ps_lstart(pid: u32) -> Option<String> {
    let mut child = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PS_TIMEOUT;
    while child.try_wait().ok()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let words: Vec<&str> = out.split_whitespace().collect();
    (!words.is_empty()).then(|| words.join("_"))
}

//! Running the fixed remote commands over ssh (Python `remote.ssh`,
//! `remote.call`, `control_options`): the paths probe (20 s), `herdr plugin
//! list` (10 s) and `inbox export|apply` with its request on stdin (10 s,
//! with ssh connection sharing). The target and the command are checked
//! types ([`SafeTarget`], [`FixedCommand`]); data only ever goes on stdin.
//! The one process ever killed here is the ssh this function started, when
//! its time is up (Python `subprocess.run(timeout=...)`).

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::remote::command::{FixedCommand, SafeTarget};
use crate::store::mirror::CallResult;

/// The options every ssh gets (Python `SSH_OPTIONS`).
pub const SSH_OPTIONS: [&str; 8] = [
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=2",
];
/// The paths probe's timeout (Python `probe`'s `timeout=20`).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// Every other call's (Python `CALL_TIMEOUT`).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// How this process runs ssh.
#[derive(Clone, Debug)]
pub struct SshEnv {
    /// The ssh client (`$INBOX_SSH`, else `ssh`).
    pub program: OsString,
    /// `$XDG_RUNTIME_DIR`, where the control socket goes.
    pub runtime_dir: Option<PathBuf>,
    /// The home directory (`~/.ssh` is the fallback).
    pub home: PathBuf,
}

/// ssh connection sharing, with a socket path short enough for the unix
/// limit (`%C` is a 40-character hash): under the runtime directory, else
/// `~/.ssh`; off when even that is too long or missing (Python
/// `control_options`).
pub fn control_options(env: &SshEnv) -> Vec<String> {
    let base = env
        .runtime_dir
        .clone()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| env.home.join(".ssh"));
    let path = base.join("hinbox-%C");
    let len = path.to_string_lossy().chars().count();
    if len - 2 + 40 > 100 || !base.is_dir() {
        return vec!["-o".into(), "ControlMaster=no".into()];
    }
    vec![
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", path.to_string_lossy()),
        "-o".into(),
        "ControlPersist=60".into(),
    ]
}

/// Runs `command` on `target`: with `stdin`, an inbox call (connection
/// sharing on, [`CALL_TIMEOUT`]); without, the probe ([`PROBE_TIMEOUT`])
/// or `plugin list` ([`CALL_TIMEOUT`]). A timeout is exit 124, a client
/// that cannot start exit 127, as Python reports them.
pub fn run(
    env: &SshEnv,
    target: &SafeTarget,
    command: &FixedCommand,
    stdin: Option<&str>,
) -> CallResult {
    let mut args: Vec<String> = SSH_OPTIONS.iter().map(|s| s.to_string()).collect();
    if stdin.is_some() {
        args.extend(control_options(env));
    }
    let timeout = timeout_of(command, stdin);
    args.extend(["--".into(), target.as_str().into(), command.as_str().into()]);
    // Python's call() sends its request even when it is empty; ssh() sends none
    exec(&env.program, &args, stdin, timeout, stdin.is_none())
}

/// How long `command` may take: only the paths probe, sent without a request,
/// gets [`PROBE_TIMEOUT`].
fn timeout_of(command: &FixedCommand, stdin: Option<&str>) -> Duration {
    if stdin.is_none() && *command == FixedCommand::probe() {
        PROBE_TIMEOUT
    } else {
        CALL_TIMEOUT
    }
}

/// Starts `program args`, writes `stdin`, and collects its output within
/// `timeout`: the process must have exited AND its pipes closed by then
/// (a descendant holding a pipe does not keep the call waiting), else it is
/// killed (if still running) and the call reads as 124. `partial`: a
/// timeout keeps the stdout read so far, else it is dropped (`call()`).
/// Python's `ssh()` gets that partial stdout as bytes, so a timed-out probe
/// fails its pull there; here the partial paths are parsed (and validated).
pub fn exec(
    program: &OsString,
    args: &[String],
    stdin: Option<&str>,
    timeout: Duration,
    partial: bool,
) -> CallResult {
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CallResult {
                code: 127,
                stdout: String::new(),
                stderr: e.to_string(),
            };
        }
    };
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || drop(pipe.write_all(text.as_bytes())));
    }
    // the pipes are read into buffers; each reader says when its pipe closed
    let (done, closed) = mpsc::channel::<()>();
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (into, done) = (Arc::clone(&buf), done.clone());
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            if let Some(mut p) = pipe {
                while let Ok(n) = p.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    into.lock()
                        .map(|mut b| b.extend_from_slice(&chunk[..n]))
                        .ok();
                }
            }
            let _ = done.send(());
        });
        buf
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let text = |b: &Arc<Mutex<Vec<u8>>>| {
        b.lock()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };
    // Python's `run(timeout=)`: the deadline covers the process AND its output: a
    // descendant still holding a pipe open does not keep the call waiting
    let deadline = Instant::now() + timeout;
    let mut status = None;
    while status.is_none() && Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(s)) => status = Some(s),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    let mut open = 2;
    while status.is_some() && open > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        match closed.recv_timeout(left) {
            Ok(()) => open -= 1,
            Err(_) => break,
        }
    }
    match status {
        Some(status) if open == 0 => CallResult {
            // Python's returncode: the exit code, or minus the signal that ended it
            code: status
                .code()
                .unwrap_or_else(|| -status.signal().unwrap_or(0)),
            stdout: text(&out),
            stderr: text(&err),
        },
        _ => {
            if status.is_none() {
                let _ = child.kill(); // our own child, past its time
                let _ = child.wait();
            }
            CallResult {
                code: 124,
                stdout: if partial { text(&out) } else { String::new() },
                stderr: format!("timed out after {} s", timeout.as_secs()),
            }
        }
    }
}

#[cfg(test)]
mod tests;

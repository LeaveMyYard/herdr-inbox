//! Starting a worker process detached, and watching a successor this
//! process started. The only process ever signalled is such a successor:
//! still held here as an unreaped [`Child`], spawned in a process group of
//! its own, so neither its pid nor its group id can name anyone else's.
//!
//! Two ways to detach. A process in a terminal (the CLI, an MCP server in a
//! pane) starts this binary with [`spawn_for_session`], and the child leaves
//! the terminal's session itself: closing the pane must not end it. A worker
//! starts its successors with [`spawn_detached`]: it has no terminal to leave,
//! and [`watch`] needs them in a group of their own.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

/// How often [`watch`] looks.
pub const POLL: Duration = Duration::from_millis(50);

/// `program args…` in `cwd`, detached in a process group of its own, with
/// exactly `env` as its environment, stdin and stdout on `/dev/null`, stderr
/// to `stderr` (else `/dev/null`). Python's `Popen(close_fds=True)`: Rust
/// closes every other descriptor on exec already. This is a new process
/// group, not a new session, so a caller's controlling terminal stays: only
/// for a caller with none, which is a worker ([`enter_own_session`]).
pub fn spawn_detached(
    program: &OsStr,
    args: &[OsString],
    cwd: Option<&Path>,
    env: &[(OsString, OsString)],
    stderr: Option<File>,
) -> io::Result<Child> {
    command(program, args, cwd, env, stderr)
        .process_group(0)
        .spawn()
}

/// [`spawn_detached`] for this binary's `worker` and `deliver`, which call
/// [`enter_own_session`] first (Python's `start_new_session=True`). The child
/// stays out of a group of its own: a group leader cannot start a session,
/// and std can only make one with `unsafe`, which this crate forbids.
pub fn spawn_for_session(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
    stderr: Option<File>,
) -> io::Result<Child> {
    command(program, args, None, env, stderr).spawn()
}

/// Leaves the session and the controlling terminal this process was started
/// in, so that closing them does not end it. It fails, harmlessly, for a
/// process that leads a group already: one Python started in a session of its
/// own, a worker's successor, or a command a shell runs as a job.
pub fn enter_own_session() {
    let _ = rustix::process::setsid();
}

fn command(
    program: &OsStr,
    args: &[OsString],
    cwd: Option<&Path>,
    env: &[(OsString, OsString)],
    stderr: Option<File>,
) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)));
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    cmd.stderr(stderr.map_or_else(Stdio::null, Stdio::from));
    cmd
}

/// Lets `child` run on: a thread reaps it when it exits, so a long-lived
/// worker leaves no zombie per process it started (Python's `subprocess`
/// reaps finished children on the next `Popen`).
pub fn reap_later(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Kills `child`'s whole group (a launcher's children too) and reaps it.
/// `child` was spawned by [`spawn_detached`] (it leads its group) and has
/// not been waited for, so its pid and group id are still its own.
fn kill_group(mut child: Child) {
    let _ = kill_process_group(Pid::from_child(&child), Signal::KILL);
    let _ = child.kill(); // the leader itself, should the group be gone
    let _ = child.wait();
}

/// Watches the successor `child` for up to `wait`: true as soon as `done`
/// (it runs on, [`reap_later`]); if `child` exits first, `gone()`. Neither
/// in time: `done` is asked once more (a worker that took the lock at the
/// last moment is never killed), then `child` is killed with its group and
/// the answer is false.
pub fn watch(
    mut child: Child,
    wait: Duration,
    done: impl Fn(&Child) -> bool,
    gone: impl Fn() -> bool,
) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if done(&child) {
            reap_later(child);
            return true;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return gone();
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(POLL);
    }
    if done(&child) {
        reap_later(child);
        return true;
    }
    kill_group(child);
    false
}

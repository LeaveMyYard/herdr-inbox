//! A worker started from a terminal outlives that terminal (Python's
//! `start_new_session=True`). A real pseudo-terminal, as a herdr pane has:
//! a shell leads the session, runs `worker --ensure` and stays; the test then
//! ends that session, as closing the pane does: the terminal goes, and every
//! process left in the session is hung up. The signal is limited to the one
//! session this script made (`pkill -s`, its leader's pid), so a process that
//! left it, and anything of anyone else's, is out of reach.

use std::fs;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use herdr_agent_kit::agent_procs::pid_alive;
use herdr_inbox::worker::handover::holder_path;
use herdr_inbox::worker::lock::is_running;
use rustix::process::{Pid, getsid};

use super::wait_for_the_end;
use crate::World;

/// Runs `sh -c <argv[1]>` as the session leader of a new pseudo-terminal and
/// ends that session once the file `argv[2]` exists.
const PANE: &str = r#"
import os, pty, select, subprocess, sys, time
pid, fd = pty.fork()
if pid == 0:
    os.execv("/bin/sh", ["sh", "-c", sys.argv[1]])
end = time.time() + 30
while not os.path.exists(sys.argv[2]) and time.time() < end:
    if select.select([fd], [], [], 0.05)[0]:
        try:
            os.read(fd, 4096)
        except OSError:
            break
os.close(fd)
subprocess.run(["pkill", "-HUP", "-s", str(pid)])
os.waitpid(pid, 0)
"#;

impl World {
    /// A pane running `command`, closed when `close` appears.
    fn pane(&self, command: &str, close: &Path) -> Child {
        let exe = env!("CARGO_BIN_EXE_herdr-inbox");
        let cmd = self.command(&[]);
        let args: Vec<_> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(ToOwned::to_owned)))
            .collect();
        let mut py = std::process::Command::new("python3");
        py.args(["-c", PANE, &command.replace("EXE", exe)])
            .arg(close)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in args {
            if let Some(v) = v {
                py.env(k, v);
            }
        }
        py.env("HERDR_INBOX_WORKER_IDLE_EXIT", "6");
        py.spawn().unwrap()
    }
}

/// The pid in the worker's holder record, once it holds the lock.
fn holder_pid(lock: &Path) -> Option<u32> {
    let text = fs::read_to_string(holder_path(lock)).ok()?;
    text.split_whitespace().nth(1)?.parse().ok()
}

#[test]
fn a_worker_started_from_a_pane_outlives_the_pane() {
    let w = World::new();
    let close = w.dir.path().join("close-the-pane");
    let mut pane = w.pane("EXE worker --ensure; sleep 30", &close);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !(is_running(&w.lock()) && holder_pid(&w.lock()).is_some()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let worker = holder_pid(&w.lock()).expect("the pane started a worker");
    let pid = Pid::from_raw(i32::try_from(worker).unwrap()).unwrap();
    let session = getsid(Some(pid)).unwrap();
    fs::write(&close, "").unwrap();
    assert_eq!(pane.wait().unwrap().code(), Some(0), "the pane closed");
    std::thread::sleep(Duration::from_millis(1500));
    let alive = pid_alive(Some(worker)) && is_running(&w.lock());
    wait_for_the_end(&w.lock());
    assert!(alive, "the worker died with its pane");
    assert_eq!(session, pid, "the worker leads a session of its own");
}

#[test]
fn the_detached_sender_leaves_the_panes_session() {
    let mut w = World::new();
    let qid = w.ask();
    w.store.set_answer(qid, "yes", None).unwrap();
    w.fake.set(|k| k.land = "none"); // typed, never seen: it waits for the transcript
    let (close, pidfile) = (w.dir.path().join("close"), w.dir.path().join("pid"));
    let line = format!(
        "HERDR_INBOX_NO_WORKER=1 EXE deliver {qid} --force & echo $! > '{}'; sleep 30",
        pidfile.display()
    );
    let mut pane = w.pane(&line, &close);
    let deadline = Instant::now() + Duration::from_secs(20);
    let sender = loop {
        if let Some(pid) = fs::read_to_string(&pidfile)
            .ok()
            .and_then(|t| t.trim().parse().ok())
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "the pane started no sender");
        std::thread::sleep(Duration::from_millis(20));
    };
    let pid = Pid::from_raw(sender).unwrap();
    let mut own = false;
    while !own && Instant::now() < deadline {
        own = getsid(Some(pid)).is_ok_and(|sid| sid == pid);
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(&close, "").unwrap();
    assert_eq!(pane.wait().unwrap().code(), Some(0), "the pane closed");
    assert!(own, "the sender leads a session of its own");
}

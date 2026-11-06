//! The core worker's life, as real processes: keyed on `herdr-core`, alive while
//! the core is, gone soon after it. The core is a socket somebody listens on, and
//! no `hc` is on `PATH`: nothing here can reach a real one.

use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use herdr_agent_kit::internal::plugin_paths::inbox::server_key;
use herdr_agent_kit::plugin_env::LockFile;

#[path = "support/exe.rs"]
mod exe;

const BIN: &str = env!("CARGO_BIN_EXE_herdr-inbox");

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("c.sock")
    }

    fn lock(&self) -> PathBuf {
        let key = server_key("herdr-core".as_ref());
        assert_eq!(key, "0fd11ed85d43");
        self.dir.path().join(format!("worker-{key}.lock"))
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path())
            .env("XDG_STATE_HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path())
            .env("HERDR_INBOX_DB", self.dir.path().join("inbox.db"))
            .env("HERDR_BIN_PATH", "/nonexistent/herdr-cli")
            .env("HC_SOCKET", self.socket())
            .env("HERDR_INBOX_WORKER_TICK", "0.05")
            .env("HERDR_INBOX_WORKER_IDLE_EXIT", "0.1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn worker(&self) -> Child {
        let mut command = self.command(&["worker"]);
        command.env("HERDR_SOCKET_PATH", "herdr-core");
        command.spawn().unwrap()
    }
}

fn held(lock: &Path) -> bool {
    LockFile::new(lock).held_elsewhere()
}

fn until(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < end, "{what}");
        sleep(Duration::from_millis(20));
    }
}

fn exits(child: &mut Child, limit: Duration) {
    until("the worker did not exit", limit, || {
        child.try_wait().unwrap().is_some()
    });
}

#[test]
fn the_core_worker_outlives_idleness_with_the_core_and_ends_after_it() {
    let rig = Rig::new();
    let core = UnixListener::bind(rig.socket()).unwrap();
    let mut worker = rig.worker();
    until("no lock", Duration::from_secs(10), || held(&rig.lock()));
    sleep(Duration::from_millis(600)); // six idle exits
    assert_eq!(worker.try_wait().unwrap(), None, "idle, and the core is up");
    drop(core);
    fs::remove_file(rig.socket()).unwrap();
    exits(&mut worker, Duration::from_secs(20));
    assert!(!held(&rig.lock()));
}

#[test]
fn a_core_worker_with_no_core_ends_by_itself() {
    let rig = Rig::new();
    let mut worker = rig.worker();
    exits(&mut worker, Duration::from_secs(20));
}

#[test]
fn a_second_core_worker_leaves_at_once() {
    let rig = Rig::new();
    let core = UnixListener::bind(rig.socket()).unwrap();
    let mut first = rig.worker();
    until("no lock", Duration::from_secs(10), || held(&rig.lock()));
    let mut second = rig.worker();
    exits(&mut second, Duration::from_secs(5));
    assert_eq!(first.try_wait().unwrap(), None);
    drop(core);
    fs::remove_file(rig.socket()).unwrap();
    exits(&mut first, Duration::from_secs(20));
}

#[test]
fn ensure_starts_the_core_worker_only_with_a_core_up() {
    let rig = Rig::new();
    assert!(
        rig.command(&["worker", "--ensure"])
            .status()
            .unwrap()
            .success()
    );
    sleep(Duration::from_millis(300));
    assert!(!held(&rig.lock()), "no core, so no core worker");
    let core = UnixListener::bind(rig.socket()).unwrap();
    assert!(
        rig.command(&["worker", "--ensure"])
            .status()
            .unwrap()
            .success()
    );
    until("no core worker", Duration::from_secs(10), || {
        held(&rig.lock())
    });
    drop(core);
    fs::remove_file(rig.socket()).unwrap();
    // detached, so not this test's child: it ends by the rule under test
    until("it outlived the core", Duration::from_secs(20), || {
        !held(&rig.lock())
    });
}

#[test]
fn an_install_gone_back_to_python_leaves_no_core_worker() {
    let rig = Rig::new();
    let root = rig.dir.path().join("install");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("herdr-plugin.toml"), "version = \"0\"\n").unwrap();
    fs::write(root.join("pyproject.toml"), "version = \"0\"\n").unwrap();
    fs::write(root.join("engine"), "rust\n").unwrap();
    let exe = root.join("bin/herdr-inbox");
    exe::link_or_copy(Path::new(BIN), &exe);
    let core = UnixListener::bind(rig.socket()).unwrap();
    let mut command = rig.command(&["worker"]);
    command.env("HERDR_SOCKET_PATH", "herdr-core");
    let mut worker = {
        let mut c = Command::new(&exe);
        c.args(command.get_args());
        c.env_clear();
        for (k, v) in command.get_envs() {
            if let Some(v) = v {
                c.env(k, v);
            }
        }
        c.stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    until("no lock", Duration::from_secs(10), || held(&rig.lock()));
    fs::write(root.join("engine"), "python\n").unwrap();
    exits(&mut worker, Duration::from_secs(20));
    drop(core);
}

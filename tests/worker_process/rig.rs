//! One test's world: a temporary state directory and database, a fake herdr
//! (its socket for the Rust workers; a CLI over the same socket's presence
//! for the Python ones), `uv` and `python3` on a private `PATH`, and the
//! worker processes the test started.

use std::ffi::OsString;
use std::fs::{self, File};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use herdr_agent_kit::agent_procs::pid_alive;
use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_agent_kit::internal::plugin_paths::inbox::server_key;
use herdr_inbox::worker::handover::holder_path;
use herdr_inbox::worker::lock::is_running;
use herdr_inbox::worker::spawn::{PYTHON_WORKER, python_worker_args};
use serde_json::json;
use tempfile::TempDir;

/// Every wait is bounded: a broken handover fails here, it never hangs.
pub const LIMIT: Duration = Duration::from_secs(30);

/// The Python fake herdr CLI: up while the fake socket exists.
const FAKE_HERDR: &str = r#"#!/bin/sh
echo "$$ $*" >> "$E1_HERDR_LOG"
case "$1 $2" in
"workspace list")
  if [ -S "$HERDR_SOCKET_PATH" ]; then echo '{"result": {"workspaces": []}}'
  else echo '{"error": {"code": "server_not_running", "message": "gone"}}' >&2; exit 1; fi ;;
"plugin list")
  root=$(cat "$E1_LISTED")
  printf '{"result": {"plugins": [{"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "%s"}]}}\n' "$root"
  echo "$$ plugin list answered" >> "$E1_HERDR_LOG" ;;
*) echo '{"result": {}}' ;;
esac
"#;

/// The interpreter behind `python3` (a pyenv shim needs an environment of
/// its own; the workers get a bare one).
fn real_python() -> &'static Path {
    static PY: OnceLock<PathBuf> = OnceLock::new();
    PY.get_or_init(|| {
        let out = Command::new("python3")
            .args([
                "-c",
                "import sys; print(sys.executable, '%d.%d' % sys.version_info[:2])",
            ])
            .output()
            .expect("python3 runs (the Python worker is under test too)");
        let text = String::from_utf8(out.stdout).unwrap();
        let (exe, version) = text
            .trim()
            .rsplit_once(' ')
            .expect("python3 printed its version");
        let (major, minor) = version.split_once('.').unwrap();
        let (major, minor): (u32, u32) = (major.parse().unwrap(), minor.parse().unwrap());
        assert!(
            (major, minor) >= (3, 11),
            "the Python worker needs python3 >= 3.11 (tomllib), {exe} is {version}"
        );
        PathBuf::from(exe)
    })
}

fn exe(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

pub struct Rig {
    pub dir: TempDir,
    pub db: PathBuf,
    fake: Option<FakeHerdr>,
    socket: PathBuf,
    bin: PathBuf,
    listed: PathBuf,
    ticks: PathBuf,
    children: Vec<(String, Child)>,
}

impl Rig {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        exe(&bin.join("herdr"), FAKE_HERDR);
        symlink(real_python(), bin.join("python3")).unwrap();
        // Rust's start_successor runs `uv run --quiet --frozen --no-dev --project <root> inbox worker`
        let uv = format!(
            "#!/bin/sh\n[ \"$1 $2 $3 $4 $5 $7 $8\" = 'run --quiet --frozen --no-dev --project inbox worker' ] || exit 2\n\
             echo \"$6\" >> \"$E1_UV_LOG\"\n\
             exec \"$(dirname \"$0\")/python3\" -I -c \"{PYTHON_WORKER}\" \"$6/src\" worker\n"
        );
        exe(&bin.join("uv"), &uv);
        let fake = FakeHerdr::start();
        fake.on("workspace.list", Reply::Result(json!({"workspaces": []})));
        let socket = fake.socket().to_path_buf();
        let state = dir.path().join("state");
        fs::create_dir_all(&state).unwrap();
        Self {
            db: state.join("inbox.db"),
            listed: dir.path().join("listed-root"),
            ticks: dir.path().join("ticks"),
            fake: Some(fake),
            socket,
            bin,
            dir,
            children: Vec::new(),
        }
    }

    pub fn lock(&self) -> PathBuf {
        let key = server_key(self.socket.as_os_str());
        self.db.parent().unwrap().join(format!("worker-{key}.lock"))
    }

    /// The environment every worker starts with, built from nothing.
    fn env(&self) -> Vec<(&'static str, OsString)> {
        let d = self.dir.path();
        let path = format!("{}:/usr/bin:/bin", self.bin.display());
        vec![
            ("PATH", path.into()),
            ("HOME", d.join("home").into()),
            ("XDG_STATE_HOME", d.join("xdg/state").into()),
            ("XDG_CONFIG_HOME", d.join("xdg/config").into()),
            ("XDG_CACHE_HOME", d.join("xdg/cache").into()),
            ("XDG_DATA_HOME", d.join("xdg/data").into()),
            ("CLAUDE_CONFIG_DIR", d.join("claude").into()),
            ("PYTHONDONTWRITEBYTECODE", "1".into()),
            ("HERDR_INBOX_DB", self.db.clone().into()),
            ("HERDR_SOCKET_PATH", self.socket.clone().into()),
            ("HERDR_BIN_PATH", self.bin.join("herdr").into()),
            ("HERDR_INBOX_WORKER_TICK", "0.2".into()),
            ("HERDR_INBOX_WORKER_IDLE_EXIT", "60".into()),
            (
                "HERDR_INBOX_STANDBY_MAX",
                LIMIT.as_secs().to_string().into(),
            ),
            ("E1_LISTED", self.listed.clone().into()),
            ("E1_TICKS", self.ticks.clone().into()),
            ("E1_HERDR_LOG", d.join("herdr.log").into()),
            ("E1_UV_LOG", d.join("uv.log").into()),
            ("E1_LIFETIME", "90".into()),
            ("E1_CHILD", "1".into()),
        ]
    }

    /// herdr lists the inbox at `root` from now on.
    pub fn list(&self, root: &Path) {
        fs::write(&self.listed, root.as_os_str().as_encoded_bytes()).unwrap();
        let plugins =
            json!({"plugins": [{"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": root}]});
        self.fake
            .as_ref()
            .unwrap()
            .on("plugin.list", Reply::Result(plugins));
    }

    fn start(&mut self, name: &str, mut cmd: Command, root: &Path, extra: &[(&str, &str)]) -> u32 {
        cmd.env_clear()
            .envs(self.env())
            .envs(extra.iter().copied())
            .current_dir(root);
        let err = File::create(self.dir.path().join(format!("{name}.err"))).unwrap();
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(err);
        let child = cmd.process_group(0).spawn().unwrap();
        let pid = child.id();
        self.children.push((name.into(), child));
        pid
    }

    /// `root`'s Python worker, as `ensure()` starts it (no `PYTHONPATH`; its
    /// `src` in argv).
    pub fn start_python(&mut self, name: &str, root: &Path) -> u32 {
        self.start_python_with(name, root, &[])
    }

    /// [`Rig::start_python`] with more variables (which the workers it
    /// starts inherit).
    pub fn start_python_with(&mut self, name: &str, root: &Path, extra: &[(&str, &str)]) -> u32 {
        let mut cmd = Command::new(self.bin.join("python3"));
        cmd.args(python_worker_args(&root.join("src")));
        self.start(name, cmd, root, extra)
    }

    /// A Rust worker of `revision` in `root`.
    pub fn start_rust(
        &mut self,
        name: &str,
        root: &Path,
        revision: i64,
        extra: &[(&str, &str)],
    ) -> u32 {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.arg("worker");
        let revision = revision.to_string();
        let mut extra = extra.to_vec();
        extra.push(("E1_REVISION", &revision));
        self.start(name, cmd, root, &extra)
    }

    /// The holder record as written: `(revision, pid)`.
    pub fn holder(&self) -> Option<(i64, u32)> {
        let text = fs::read_to_string(holder_path(&self.lock())).ok()?;
        let mut words = text.split_whitespace();
        Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?))
    }

    /// A worker holds the lock and its record names `(revision, pid)`.
    pub fn held_by(&self, revision: i64, pid: u32) -> bool {
        is_running(&self.lock()) && self.holder() == Some((revision, pid))
    }

    /// Every tick's pid, in order.
    pub fn tick_sequence(&self) -> Vec<u32> {
        let text = fs::read_to_string(&self.ticks).unwrap_or_default();
        text.lines()
            .map(|l| l.split(' ').next().unwrap().parse().unwrap())
            .collect()
    }

    /// The install root `pid` ticked from (its cwd).
    pub fn tick_root(&self, pid: u32) -> Option<PathBuf> {
        let text = fs::read_to_string(&self.ticks).unwrap_or_default();
        let line = text
            .lines()
            .find(|l| l.split(' ').next() == Some(&pid.to_string()))?;
        line.split_once(' ').map(|(_, root)| PathBuf::from(root))
    }

    /// The roots the fake `uv` was asked to run a worker of.
    pub fn uv_roots(&self) -> Vec<PathBuf> {
        let text = fs::read_to_string(self.dir.path().join("uv.log")).unwrap_or_default();
        text.lines().map(PathBuf::from).collect()
    }

    /// A Python worker has herdr's list of plugins: it decides from that first
    /// answer, once, whether it is herdr's install. The answer counts, not the
    /// call: a [`Rig::list`] after the call and before the fake reads the
    /// listed root tells the worker it is not herdr's install, for good.
    pub fn python_asked_for_its_root(&self) -> bool {
        self.herdr_called(" plugin list answered")
    }

    /// The fake herdr CLI logged a line ending in `suffix`.
    pub fn herdr_called(&self, suffix: &str) -> bool {
        fs::read_to_string(self.dir.path().join("herdr.log"))
            .is_ok_and(|log| log.lines().any(|l| l.ends_with(suffix)))
    }

    /// The fake herdr CLI with `args`, in the workers' environment.
    pub fn herdr_cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(self.bin.join("herdr"));
        cmd.args(args).env_clear().envs(self.env());
        cmd
    }

    /// The file the fake herdr CLI reads the listed root from.
    pub fn listed_file(&self) -> &Path {
        &self.listed
    }

    /// The pids that ticked, in order of their first tick.
    pub fn ticked(&self) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for pid in self.tick_sequence() {
            if !out.contains(&pid) {
                out.push(pid);
            }
        }
        out
    }

    /// The exit status of a child this test started, once it exited.
    pub fn exited(&mut self, pid: u32) -> Option<ExitStatus> {
        let (_, child) = self.children.iter_mut().find(|(_, c)| c.id() == pid)?;
        child.try_wait().ok().flatten()
    }

    /// Waits for a child this test started to exit; its status, or a
    /// panic naming `what` with the logs.
    pub fn must_exit(&mut self, pid: u32, what: &str) -> ExitStatus {
        match self.wait_exit(pid) {
            Some(status) => status,
            None => panic!("{what}: pid {pid} still runs\n{}", self.logs()),
        }
    }

    /// Waits for a child this test started to exit; its status.
    pub fn wait_exit(&mut self, pid: u32) -> Option<ExitStatus> {
        let deadline = Instant::now() + LIMIT;
        while Instant::now() < deadline {
            if let Some(status) = self.exited(pid) {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    /// The herdr server goes away (its socket with it).
    pub fn server_goes_away(&mut self) {
        self.fake = None;
    }

    /// The test's end: the server goes away, and every worker in `pids`
    /// (also ones its children started) must then exit by itself, the lock
    /// left free. Nothing is signalled here.
    pub fn finish(mut self, pids: &[u32]) {
        self.server_goes_away();
        let deadline = Instant::now() + LIMIT;
        let own: Vec<u32> = self.children.iter().map(|(_, c)| c.id()).collect();
        let gone = |rig: &mut Self, pid: u32| {
            if own.contains(&pid) {
                rig.exited(pid).is_some()
            } else {
                !pid_alive(Some(pid))
            }
        };
        while Instant::now() < deadline && !pids.iter().all(|p| gone(&mut self, *p)) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let left: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|p| !gone(&mut self, *p))
            .collect();
        assert!(
            left.is_empty(),
            "workers outlived their server: {left:?}\n{}",
            self.logs()
        );
        assert!(!is_running(&self.lock()), "the lock is free");
    }

    /// Every worker's stderr and the Python workers' herdr calls, for a
    /// failure's message.
    pub fn logs(&self) -> String {
        let calls = fs::read_to_string(self.dir.path().join("herdr.log")).unwrap_or_default();
        let tail: Vec<&str> = calls.lines().rev().take(40).collect();
        let mut out = format!(
            "--- herdr calls (last 40, newest first)\n{}\n",
            tail.join("\n")
        );
        for (name, _) in &self.children {
            let text =
                fs::read_to_string(self.dir.path().join(format!("{name}.err"))).unwrap_or_default();
            out.push_str(&format!("--- {name}\n{text}"));
        }
        out
    }
}

impl Drop for Rig {
    /// Children this test started and still holds, unreaped: their pids are
    /// still theirs, so only they are killed (never their process groups,
    /// never a worker one of them started).
    ///
    /// Then the server goes away while the fake herdr CLI still exists, and
    /// the drop waits (bounded) for the lock to be free: a worker one of them
    /// started, left behind by a failed test, ends by the server-gone rule
    /// before the temporary directory (and the CLI that says "gone") is
    /// deleted, instead of lingering until its idle exit.
    fn drop(&mut self) {
        for (_, child) in &mut self.children {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        self.server_goes_away();
        let deadline = Instant::now() + LIMIT;
        while is_running(&self.lock()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Waits up to [`LIMIT`] for `cond`.
pub fn wait_for(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + LIMIT;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// `pid` runs the Python worker (its command line).
pub fn is_python_worker(pid: u32) -> bool {
    let out = Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains("herdr_inbox.cli")
}

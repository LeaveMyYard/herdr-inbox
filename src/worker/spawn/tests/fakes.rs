//! Stand-ins for what a worker starts: `uv`, `python3`, and a Rust worker
//! (Python tests/test_successor.py's FAKE_RUST). Every one logs its call and
//! exits on its own within seconds: no test ever needs to signal one, and a
//! failed test leaves nothing running for long.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tempfile::TempDir;

use crate::worker::spawn::ENGINE_FILE;
use crate::worker::{Waits, WorkerEnv};

/// The interpreter behind `python3` here (a pyenv shim needs its own
/// environment; the fakes run with a bare one).
pub fn real_python() -> &'static Path {
    static PY: OnceLock<PathBuf> = OnceLock::new();
    PY.get_or_init(|| {
        let out = std::process::Command::new("python3")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .expect("python3 runs");
        PathBuf::from(String::from_utf8(out.stdout).unwrap().trim())
    })
}

/// A Rust worker stand-in (`FAKE_MODE`): `lock` takes the lock (exits 0 if
/// taken) and holds it 3 s; `exit` has no `worker` (exits 2); `hang` never
/// takes it (sleeps 6 s); `standby` writes a standby record of revision 2.
const FAKE_RUST: &str = r#"
import fcntl, os, sys, time
mode = os.environ.get("FAKE_MODE", "lock")
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write("rust|%s|%s|%s|%d\n" % (sys.argv[0], " ".join(sys.argv[1:]), os.getcwd(), os.getpid()))
if mode == "exit":
    sys.exit(2)
if mode == "hang":
    time.sleep(6)
    sys.exit(0)
lock = os.environ["FAKE_LOCK"]
if mode == "standby":
    with open(lock + ".standby", "w") as fh:
        fh.write("2 %d %.3f -\n" % (os.getpid(), time.time()))
    time.sleep(3)
    sys.exit(0)
fh = open(lock, "a")
try:
    fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    sys.exit(0)
time.sleep(3)
"#;

/// Logs `<name>|<args>|<cwd>`, then one `venv <var>` line per `UV_*` or
/// `VIRTUAL_ENV` it inherited.
const LOGGER: &str = "#!/bin/sh\n\
printf '%s|%s|%s\\n' \"$(basename \"$0\")\" \"$*\" \"$(pwd)\" >> \"$FAKE_LOG\"\n\
env | grep -E '^(UV_|VIRTUAL_ENV=)' | sed 's/=.*//; s/^/venv /' >> \"$FAKE_LOG\"\n";

/// A state directory, a bin directory first on the fakes' `PATH`, a log.
pub struct Fakes {
    pub dir: TempDir,
    pub bin: PathBuf,
    pub log: PathBuf,
}

impl Fakes {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let log = dir.path().join("calls.log");
        Self { dir, bin, log }
    }

    /// A worker environment in the state directory; the fakes inherit a
    /// venv selection that must not reach them.
    pub fn env(&self) -> WorkerEnv {
        let db = self.dir.path().join("state").join("inbox.db");
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lock_env = WorkerEnv {
            db: db.clone(),
            server: "/fake/herdr.sock".into(),
            revision: 1,
            version: "0.10.6".into(),
            waits: Waits::default(),
            spawn_env: Vec::new(),
            no_worker: false,
        };
        let lock = lock_env.lock_path();
        let path = format!("{}:/usr/bin:/bin", self.bin.display());
        let vars: [(&str, OsString); 4] = [
            ("PATH", path.into()),
            ("FAKE_LOG", self.log.clone().into()),
            ("FAKE_LOCK", lock.into()),
            ("UV_PROJECT_ENVIRONMENT", "/data/venv-0.10.5".into()),
        ];
        let vars = vars.into_iter().map(|(k, v)| (OsString::from(k), v));
        WorkerEnv {
            spawn_env: crate::worker::spawn_env(vars, "".as_ref()),
            ..lock_env
        }
    }

    /// `name` on the fakes' `PATH`, logging its calls.
    pub fn logger(&self, name: &str) -> PathBuf {
        write_exe(&self.bin.join(name), LOGGER)
    }

    /// A Rust worker stand-in at `root/rel` in `mode` (`FAKE_MODE` is set
    /// through `env`).
    pub fn rust(&self, root: &Path, rel: &str) -> PathBuf {
        let script = format!("#!{}\n{FAKE_RUST}", real_python().display());
        write_exe(&root.join(rel), &script)
    }

    /// The log's lines.
    pub fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// An install root, as the OS names it (macOS's temporary directory is
    /// behind a symlink, and a child's `pwd` is the resolved path).
    pub fn root(&self, name: &str) -> PathBuf {
        let root = self.dir.path().join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(ENGINE_FILE), "rust\n").unwrap();
        fs::canonicalize(root).unwrap()
    }
}

/// `env` with `FAKE_MODE=mode`.
pub fn with_mode(mut env: WorkerEnv, mode: &str) -> WorkerEnv {
    env.spawn_env.push(("FAKE_MODE".into(), mode.into()));
    env
}

pub fn write_exe(path: &Path, text: &str) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    path.to_path_buf()
}

/// Waits up to 10 s for `cond`.
pub fn wait_for(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    cond()
}

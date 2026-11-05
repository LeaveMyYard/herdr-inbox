//! `<exe> worker`: a Rust worker, run the way a binary's `main` will run it,
//! with duties that only record each tick (`<pid> <root>` appended to
//! `$E1_TICKS`).
//!
//! It runs in its install root (the cwd: the Python bridge starts a Rust
//! successor from its root, and a test starts it there), whose
//! `pyproject.toml` gives its version (`running_version`, as `main` will); `E1_REVISION` is its revision.
//! `E1_IDLE=1` makes it idle (nothing kept alive). It never outlives
//! `E1_LIFETIME` seconds, whatever happens to the test.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use herdr_agent_kit::herdr_api::ServerId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_inbox::store::revision::MIGRATION_REVISION;
use herdr_inbox::worker::live::{Duties, Install, Live, running_version};
use herdr_inbox::worker::run::{Ticked, run};
use herdr_inbox::worker::{Waits, WorkerEnv, spawn_env};

struct Recorder {
    ticks: Option<PathBuf>,
    root: PathBuf,
    idle: bool,
}

impl Duties for Recorder {
    fn migrate(&mut self) {}
    fn tick(&mut self) -> Ticked {
        if let Some(path) = &self.ticks
            && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(file, "{} {}", std::process::id(), self.root.display());
        }
        Ticked {
            pending: 0,
            keep_alive: !self.idle,
        }
    }
    fn pending_now(&mut self) -> usize {
        0
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn main() -> ExitCode {
    let lifetime = var("E1_LIFETIME")
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(lifetime));
        std::process::exit(3); // the kernel releases the lock with the process
    });
    let (Some(db), Some(socket)) = (var("HERDR_INBOX_DB"), var("HERDR_SOCKET_PATH")) else {
        return ExitCode::from(2);
    };
    let root = std::env::current_dir().expect("a cwd");
    let server = OsString::from(&socket);
    let env = WorkerEnv {
        db: db.into(),
        revision: var("E1_REVISION")
            .and_then(|v| v.parse().ok())
            .unwrap_or(MIGRATION_REVISION),
        version: running_version(Some(&root)),
        waits: Waits::from_vars(var),
        spawn_env: spawn_env(std::env::vars_os(), &server),
        no_worker: false,
        server,
    };
    let duties = Recorder {
        ticks: var("E1_TICKS").map(PathBuf::from),
        root: root.clone(),
        idle: var("E1_IDLE").as_deref() == Some("1"),
    };
    // herdr's CLI as the rig points HERDR_BIN_PATH at it (its logging fake), never PATH's
    let bin = var("HERDR_BIN_PATH").unwrap_or_else(|| "/nonexistent/herdr-cli".into());
    let herdr = Herdr::new(ServerId::new(&socket)).with_bin(bin);
    let exe = std::env::current_exe().expect("its path");
    let mut world = Live::new(env.clone(), herdr, Install::new(Some(root)), exe, duties);
    ExitCode::from(u8::try_from(run(&env, &mut world)).unwrap_or(1))
}

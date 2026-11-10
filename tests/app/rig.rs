//! Real processes in a private world. Every command starts from `env_clear`, with private
//! homes, dead sockets and a `PATH` that holds only this rig's stand-ins, so nothing started
//! here can find a default socket or a machine's `herdr`, `hc` or `herdr-app`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[path = "../support/exe.rs"]
pub mod exe;

/// The binary under test.
pub const BINARY: &str = env!("CARGO_BIN_EXE_herdr-inbox");

/// A private world.
pub struct Rig {
    /// Everything lives here.
    pub dir: tempfile::TempDir,
    /// `<dir>/app`.
    pub home: PathBuf,
    /// `<dir>/bin`, the whole `PATH`.
    pub bin: PathBuf,
}

impl Rig {
    /// A new one, with `herdr`, `hc` and `herdr-app` stand-ins. `herdr` and `hc` log every call
    /// and fail, `herdr-app status` exits with the number in `<dir>/app-status`.
    pub fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("app");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        for tool in ["herdr", "hc"] {
            let log = dir.path().join(format!("{tool}.log"));
            let body = format!("#!/bin/sh\necho \"$*\" >> {}\nexit 1\n", log.display());
            exe::install(&bin.join(tool), body);
        }
        let status = dir.path().join("app-status");
        let body = format!("#!/bin/sh\nread n < {}\nexit \"$n\"\n", status.display());
        exe::install(&bin.join("herdr-app"), body);
        let rig = Rig { dir, home, bin };
        rig.app_status(0);
        rig
    }

    /// What `herdr-app status` exits with from now on.
    pub fn app_status(&self, code: i32) {
        fs::write(self.dir.path().join("app-status"), format!("{code}\n")).unwrap();
    }

    /// The app data folder, as an install leaves it.
    pub fn install(&self) {
        let data = self.data();
        fs::create_dir_all(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(data.join("control.json"), br#"{"v":1,"disabled":false}"#).unwrap();
    }

    /// `<home>/data/herdr-inbox`.
    pub fn data(&self) -> PathBuf {
        self.home.join("data/herdr-inbox")
    }

    /// `exe` with only this rig's environment.
    pub fn command(&self, exe: &Path, db: &Path) -> Command {
        let d = self.dir.path();
        let mut cmd = Command::new(exe);
        cmd.env_clear()
            .env("HOME", d)
            .env("PATH", &self.bin)
            .env("XDG_STATE_HOME", d.join("state"))
            .env("XDG_CONFIG_HOME", d.join("xdg"))
            .env("XDG_RUNTIME_DIR", d.join("run"))
            .env("HERDR_APP_HOME", &self.home)
            .env("HERDR_SOCKET_PATH", d.join("no-herdr.sock"))
            .env("HC_SOCKET", d.join("no-core.sock"))
            .env("HERDR_BIN_PATH", self.bin.join("herdr"))
            .env("HERDR_INBOX_DB", db)
            .env("HERDR_PLUGIN_CONFIG_DIR", d.join("cfg"))
            .env("CLAUDE_CONFIG_DIR", d.join("claude"))
            .env("HERDR_INBOX_NO_WORKER", "1");
        cmd
    }

    /// Calls of `tool`, one line each.
    pub fn calls(&self, tool: &str) -> String {
        fs::read_to_string(self.dir.path().join(format!("{tool}.log"))).unwrap_or_default()
    }
}

/// Waits for `done`, up to `limit`.
pub fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

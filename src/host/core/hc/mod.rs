//! `hc`, herdr-core's CLI: one adapter, and bounded reads. No shell.
//!
//! **Any `hc` command starts a core if none runs**, `get` included, and no
//! option stops that. A core gives its environment to every terminal. So `hc`
//! runs only when the core's socket already accepts a connection
//! ([`socket`]), and never with the caller's `HERDR_*`, which a core started
//! in a race would keep, nor `HC_TERMINAL_ID`, which would name this caller
//! and mark things as seen (herdr-panorama `docs/protocol/core-host.md`).
//!
//! The JSON is hc's own, recorded per version in `tests/fixtures/hc/`. It
//! differs from `hc schema`, and between versions.

mod bounded;
pub mod socket;

use std::ffi::OsString;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::address;

const MAX_BYTES: usize = 2 * 1024 * 1024;

/// What the proof asks herdr-core.
pub trait Hc: Send + Sync {
    /// `hc get --json <address>`.
    fn get(&self, address: &str) -> Result<Value, &'static str>;
}

/// The real `hc`, from `PATH`.
pub struct Cli {
    binary: OsString,
    timeout: Duration,
    /// The core's socket, if its place is known.
    socket: Option<PathBuf>,
}

impl Cli {
    /// `hc` from `PATH`, for the core at `socket`.
    pub fn on_path(socket: Option<PathBuf>) -> Cli {
        Cli::new("hc".into(), Duration::from_secs(2), socket)
    }

    /// `binary`, each read bounded by `timeout`, for the core at `socket`.
    pub fn new(binary: OsString, timeout: Duration, socket: Option<PathBuf>) -> Cli {
        Cli {
            binary,
            timeout,
            socket,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(args).env_remove("HC_TERMINAL_ID");
        for (name, _) in std::env::vars_os() {
            if name.as_encoded_bytes().starts_with(b"HERDR_") {
                command.env_remove(name);
            }
        }
        command
    }

    fn read(&self, args: &[&str]) -> Result<Value, &'static str> {
        let socket = self.socket.as_deref().ok_or("no known core socket")?;
        if UnixStream::connect(socket).is_err() {
            return Err("no core is running");
        }
        let output = bounded::read(self.command(args), self.timeout, MAX_BYTES)?;
        if !output.status.success() {
            return Err("hc read failed");
        }
        serde_json::from_slice(&output.stdout).map_err(|_| "hc output is not JSON")
    }
}

impl Hc for Cli {
    fn get(&self, address: &str) -> Result<Value, &'static str> {
        if !address::valid(address) {
            return Err("invalid core address");
        }
        self.read(&["get", "--json", address])
    }
}

#[cfg(test)]
mod tests;

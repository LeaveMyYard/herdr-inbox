//! `hc`, herdr-core's CLI, read-only: one adapter, and bounded reads. No
//! shell. `HC_TERMINAL_ID` is removed from the child, so that a read neither
//! names this caller nor marks anything as seen (herdr-panorama
//! `docs/protocol/core-host.md`).
//!
//! The JSON is hc's own, recorded per version in `tests/fixtures/hc/`. It
//! differs from `hc schema`, and between versions.

mod bounded;

use std::ffi::OsString;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::address;

const MAX_BYTES: usize = 2 * 1024 * 1024;

/// What the proof asks herdr-core.
pub trait Hc: Send + Sync {
    /// `hc get --json <address>`.
    fn get(&self, address: &str) -> Result<Value, &'static str>;
    /// `hc ls --json`.
    fn ls(&self) -> Result<Value, &'static str>;
}

/// The real `hc`, from `PATH`.
pub struct Cli {
    binary: OsString,
    timeout: Duration,
}

impl Default for Cli {
    fn default() -> Self {
        Cli::new("hc".into(), Duration::from_secs(2))
    }
}

impl Cli {
    /// `binary`, each read bounded by `timeout`.
    pub fn new(binary: OsString, timeout: Duration) -> Cli {
        Cli { binary, timeout }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(args).env_remove("HC_TERMINAL_ID");
        command
    }

    fn read(&self, args: &[&str]) -> Result<Value, &'static str> {
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

    fn ls(&self) -> Result<Value, &'static str> {
        self.read(&["ls", "--json"])
    }
}

#[cfg(test)]
mod tests;

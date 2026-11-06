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
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use serde_json::Value;

use super::address;

const MAX_BYTES: usize = 2 * 1024 * 1024;

/// **The switch.** `true` types into a terminal at `done` with a plain `hc send`,
/// which still refuses `blocked`. `false` keeps `--if idle` there, and the answer
/// is left to the watcher, a hook or `check_answers`. Never `--force` either way.
pub const PLAIN_SEND_AT_DONE: bool = false;

/// How long a send may take. `hc send` waits for a person to pause and then
/// for the turn to start, under a second when nobody types (6 s and more when
/// someone does). Well under a claim's life.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// `hc`'s answer for a terminal it does not have, or one whose program ended.
pub const NO_TERMINAL: &str = "hc has no such live terminal";

/// `hc`'s exit codes (`hc --skill`).
const NOT_FOUND: i32 = 3;
const BLOCKED: i32 = 9;
const STATUS_MISMATCH: i32 = 10;
const FINISHED: i32 = 11;

/// What became of `hc send`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sent {
    /// `hc` typed it, or was stopped when it may have: only the transcript says.
    Typed,
    /// Nothing was typed: the terminal is at a prompt or dialog.
    Blocked,
    /// Nothing was typed: the terminal's status is not `idle`.
    NotIdle,
    /// Nothing was typed: the terminal or its program is gone.
    Gone,
    /// Nothing was typed: `hc` could not be asked. Says why.
    Unread(&'static str),
    /// `hc` failed in a way it did not explain as refused.
    Failed(String),
}

/// What delivery and the proof ask herdr-core.
pub trait Hc: Send + Sync {
    /// `hc get --json <address>`.
    fn get(&self, address: &str) -> Result<Value, &'static str>;
    /// The visible screen: `hc read --json <address>`'s text.
    fn screen(&self, address: &str) -> Result<String, &'static str>;
    /// `hc send` of `text` to a terminal last seen at `status`. There is no way
    /// to say `--force` here: a core terminal never gets one, whoever asks.
    fn send(&self, address: &str, text: &str, status: &str) -> Sent;
    /// The live terminals of this machine: `hc ls --json`.
    fn list(&self) -> Vec<String>;
}

/// The arguments of a send: `--if idle`, unless [`PLAIN_SEND_AT_DONE`] says
/// otherwise for `done`.
fn send_args<'a>(address: &'a str, text: &'a str, status: &str, plain: bool) -> Vec<&'a str> {
    let mut args = vec!["send", "--json"];
    if !(plain && status == "done") {
        args.extend(["--if", "idle"]);
    }
    args.extend([address, "--", text]);
    args
}

/// The real `hc`, from `PATH`.
pub struct Cli {
    binary: OsString,
    timeout: Duration,
    send_timeout: Duration,
    /// The core's socket, if its place is known.
    socket: Option<PathBuf>,
}

impl Cli {
    /// `hc` from `PATH`, for the core at `socket`. A unit test has none: it
    /// must name a stub through [`Cli::new`], so it can never reach a real core.
    pub fn on_path(socket: Option<PathBuf>) -> Cli {
        let socket = socket.filter(|_| !cfg!(test));
        Cli::new("hc".into(), Duration::from_secs(2), socket)
    }

    /// `binary`, each read bounded by `timeout`, for the core at `socket`.
    pub fn new(binary: OsString, timeout: Duration, socket: Option<PathBuf>) -> Cli {
        Cli {
            binary,
            timeout,
            send_timeout: SEND_TIMEOUT,
            socket,
        }
    }

    /// [`Cli`] with sends bounded by `timeout`.
    pub fn with_send_timeout(mut self, timeout: Duration) -> Cli {
        self.send_timeout = timeout;
        self
    }

    /// `hc` for the core at `socket`, and no other: `HC_SOCKET` makes it talk
    /// to the socket just tried, whatever its own rule would have found.
    fn command(&self, socket: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .env("HC_SOCKET", socket)
            .env_remove("HC_TERMINAL_ID");
        for (name, _) in std::env::vars_os() {
            if name.as_encoded_bytes().starts_with(b"HERDR_") {
                command.env_remove(name);
            }
        }
        command
    }

    /// The core's socket, once it accepts a connection.
    fn core(&self) -> Result<&Path, &'static str> {
        let socket = self.socket.as_deref().ok_or("no known core socket")?;
        match UnixStream::connect(socket) {
            Ok(_) => Ok(socket),
            Err(_) => Err("no core is running"),
        }
    }

    fn output(&self, args: &[&str]) -> Result<Output, &'static str> {
        bounded::read(self.command(self.core()?, args), self.timeout, MAX_BYTES)
    }

    fn json(&self, args: &[&str]) -> Result<Value, &'static str> {
        let output = self.output(args)?;
        if matches!(output.status.code(), Some(NOT_FOUND | FINISHED)) {
            return Err(NO_TERMINAL);
        }
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
        self.json(&["get", "--json", address])
    }

    fn screen(&self, address: &str) -> Result<String, &'static str> {
        if !address::valid(address) {
            return Err("invalid core address");
        }
        let read = self.json(&["read", "--json", address])?;
        let text = read["text"].as_str().ok_or("hc read has no text")?;
        Ok(text.to_owned())
    }

    fn list(&self) -> Vec<String> {
        let Ok(listed) = self.json(&["ls", "--json"]) else {
            return Vec::new();
        };
        let terminals = listed["terminals"].as_array().into_iter().flatten();
        let live = terminals.filter(|t| t["state"] == "live");
        let addresses = live.filter_map(|t| t["address"].as_str());
        addresses
            .filter(|a| address::valid(a) && !address::remote(a))
            .map(str::to_owned)
            .collect()
    }

    fn send(&self, address: &str, text: &str, status: &str) -> Sent {
        if !address::valid(address) {
            return Sent::Unread("invalid core address");
        }
        let socket = match self.core() {
            Ok(socket) => socket,
            Err(why) => return Sent::Unread(why),
        };
        let args = send_args(address, text, status, PLAIN_SEND_AT_DONE);
        let run = bounded::read(self.command(socket, &args), self.send_timeout, MAX_BYTES);
        match run {
            Err(why @ "hc unavailable") => Sent::Unread(why),
            Err(_) => Sent::Typed, // stopped part way: it may have gone
            Ok(output) if output.status.success() => Sent::Typed,
            Ok(output) => match output.status.code() {
                Some(BLOCKED) => Sent::Blocked,
                Some(STATUS_MISMATCH) => Sent::NotIdle,
                Some(NOT_FOUND | FINISHED) => Sent::Gone,
                code => Sent::Failed(failure(code, &output.stderr)),
            },
        }
    }
}

/// `hc`'s own `{error: {message}}`, or its exit code.
fn failure(code: Option<i32>, stderr: &[u8]) -> String {
    let parsed: Option<Value> = serde_json::from_slice(stderr).ok();
    let message = parsed.as_ref().and_then(|v| v["error"]["message"].as_str());
    match (message, code) {
        (Some(message), _) => format!("hc send failed: {message}"),
        (None, Some(code)) => format!("hc send failed: exit {code}"),
        (None, None) => "hc send failed: killed".into(),
    }
}

#[cfg(test)]
mod tests;

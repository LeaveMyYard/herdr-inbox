//! The detached sender (Python `service.spawn_cli`, `tui.spawn_delivery`,
//! `remote_api.spawn_delivery`): `<exe> deliver <ids> [--force]`, which enters
//! a session of its own as it starts ([`enter_own_session`]), so a popup pane
//! that closes does not end it. Stderr goes to `worker.log`. The caller returns at once;
//! the answers are recorded already, so a sender that cannot start only
//! leaves them to the worker's retry.
//!
//! [`enter_own_session`]: super::enter_own_session

use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::io;
use std::path::Path;

use super::child::{reap_later, spawn_for_session};
use crate::worker::spawn_env;

/// The sender's arguments (Python `deliver <ids> [--force]`).
pub fn deliver_args(qids: &[i64], force: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["deliver".into()];
    args.extend(qids.iter().map(|q| OsString::from(q.to_string())));
    if force {
        args.push("--force".into());
    }
    args
}

/// Starts `exe deliver` for `qids`, logging to `state_dir/worker.log`. `exe`
/// `None`: this process does not know its binary, which is an error here.
/// `server` is the herdr server to deliver on, as `ensure` passes one: `deliver`
/// leaves another server's answer to that server's worker, dropping any
/// `--force`. `None`: the inherited one.
pub fn start_sender(
    exe: Option<&Path>,
    state_dir: &Path,
    qids: &[i64],
    force: bool,
    server: Option<&OsStr>,
) -> io::Result<()> {
    let exe = exe.ok_or_else(|| io::Error::other("this binary's path is unknown"))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(state_dir.join("worker.log"))
        .ok();
    let env = spawn_env(std::env::vars_os(), server.unwrap_or_default());
    let args = deliver_args(qids, force);
    let child = spawn_for_session(exe.as_os_str(), &args, &env, stderr)?;
    reap_later(child);
    Ok(())
}

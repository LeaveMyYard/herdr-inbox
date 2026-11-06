//! The binary's `worker` subcommand and every process's "make sure a worker
//! runs" (Python `cli`'s `worker [--ensure]`, `worker.ensure`,
//! `service._ensure_worker`): the one place the worker's world is read
//! from the process environment ([`worker_env`] is its pure half).

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;
use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::duties::live::LiveDuties;
use super::live::{Install, Live, own_checkout, running_version};
use super::remote::RemoteEnv;
use super::remote::ssh::SshEnv;
use super::run::run;
use super::spawn::{ensure, enter_own_session};
use super::{Waits, WorkerEnv, spawn_env};
use crate::claude::Transcripts;
use crate::store::revision::MIGRATION_REVISION;
use crate::tabs::settings::PLUGIN_ID;

/// Turns starting workers off (Python `HERDR_INBOX_NO_WORKER`, the test suites).
pub const NO_WORKER_VAR: &str = "HERDR_INBOX_NO_WORKER";

/// The worker's world for `server` (`None`: this process's herdr server, as
/// the kit resolves it), from `env`, the process's variables `vars` and its
/// executable `exe` (whose checkout gives the running version).
pub fn worker_env(
    env: &PluginEnv,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    server: Option<&OsStr>,
    exe: &Path,
) -> WorkerEnv {
    let server = server
        .filter(|s| !s.is_empty())
        .map_or_else(|| inbox::server_id(env), OsStr::to_owned);
    let own = own_checkout(exe);
    WorkerEnv {
        db: inbox::db_path(env),
        revision: MIGRATION_REVISION,
        version: running_version(own.as_deref()),
        waits: Waits::from_vars(|name| env.var_os(name).map(|v| v.to_string_lossy().into_owned())),
        spawn_env: spawn_env(vars, &server),
        no_worker: env.var_os(NO_WORKER_VAR).is_some(),
        server,
    }
}

/// The install of the worker running `exe`: its checkout, so a dev worktree
/// never follows the root herdr lists into other code, or a downgrade.
fn install_of(exe: &Path) -> Install {
    Install::new(own_checkout(exe))
}

/// This binary, for starting a worker.
pub fn own_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("herdr-inbox"))
}

/// Starts a worker for `server` (`None`: this process's) unless one runs
/// (Python `_ensure_worker`: never fails the caller). True if one started.
pub fn ensure_worker(server: Option<&str>) -> bool {
    ensure_worker_in(&PluginEnv::from_env(), server, &own_exe())
}

/// [`ensure_worker`] in `env`, starting `exe`: for a process that holds both.
pub fn ensure_worker_in(env: &PluginEnv, server: Option<&str>, exe: &Path) -> bool {
    let wenv = worker_env(env, std::env::vars_os(), server.map(OsStr::new), exe);
    ensure(&wenv, exe)
}

/// `herdr-inbox worker [--ensure]`: the exit code. `--ensure` starts one in
/// the background unless it runs and returns; without it this process is
/// the worker (it exits 0 when another worker runs).
pub fn main(args: &[String]) -> i32 {
    match args {
        [] => serve(),
        [flag] if flag == "--ensure" => {
            ensure_worker(None);
            0
        }
        _ => {
            let _ = writeln!(std::io::stderr(), "usage: herdr-inbox worker [--ensure]");
            2
        }
    }
}

fn serve() -> i32 {
    enter_own_session();
    let env = PluginEnv::from_env();
    let exe = own_exe();
    let wenv = worker_env(&env, std::env::vars_os(), None, &exe);
    let herdr = crate::trace::observed(Herdr::from_env());
    let config = ConfigFile::resolve(&env, || herdr.plugin_config_dir(PLUGIN_ID).ok().flatten());
    let remote = RemoteEnv {
        db: wenv.db.clone(),
        herdr: herdr.clone(),
        ssh: SshEnv {
            program: env
                .var_os("INBOX_SSH")
                .map_or_else(|| OsString::from("ssh"), OsStr::to_owned),
            runtime_dir: env.var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
            home: env
                .var_os("HOME")
                .map_or_else(|| PathBuf::from("/"), PathBuf::from),
        },
        config_dir: config.path().and_then(Path::parent).map(Path::to_path_buf),
    };
    let duties = LiveDuties::new(
        wenv.clone(),
        herdr.clone(),
        Transcripts::from_env(),
        config,
        remote,
    );
    let mut world = Live::new(wenv.clone(), herdr, install_of(&exe), exe, duties);
    run(&wenv, &mut world)
}

#[cfg(test)]
mod tests;

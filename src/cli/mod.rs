//! The `inbox` command line (Python `cli.main`): so far `list`, `answer`,
//! `dismiss` and `open`. Output, exit codes and usage errors are Python's, word for
//! word (the CLI golden, `tests/fixtures/cli.json`).
//!
//! - [`args`]: argparse's parsing, help and usage errors.
//! - [`specs`]: the subcommands' arguments and help.
//! - [`list`], [`answer`], [`open`]: the commands.
//! - [`live`]: the real database, herdr and delivery behind [`Io`].

pub mod answer;
pub mod args;
pub mod list;
pub mod live;
pub mod open;
pub mod specs;

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;

use crate::claude::Transcripts;
use crate::service::Delivery;
use crate::service::own::Me;
use crate::store::revision::MIGRATION_REVISION;
use crate::store::revision_wanted::{unix_now, want};
use crate::store::{OpenError, Store};
use crate::surface::Surface;
use args::parse::{Args, Parsed, parse};

/// What `NotReady` says (Python `cli.main`'s `SchemaNotReady` branch): the
/// CLI never migrates; the worker does, and is started.
pub const NOT_MIGRATED: &str = "inbox: the inbox database is not migrated to this version yet; \
    the delivery worker (started now) migrates it: run this again in a few seconds";

/// How long a statement waits for SQLite's write lock (Python `Store()`'s
/// `timeout=15.0`).
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(15);

/// Everything a command reads and writes besides its arguments.
pub struct Io<'a> {
    /// stdout.
    pub out: &'a mut dyn Write,
    /// stderr.
    pub err: &'a mut dyn Write,
    /// The database.
    pub db_path: &'a Path,
    /// The inbox's state directory (session markers).
    pub state_dir: &'a Path,
    /// Claude Code's transcripts.
    pub files: &'a Transcripts,
    /// Who runs this command.
    pub me: Me<'a>,
    /// Delivery and the worker.
    pub delivery: &'a mut dyn Delivery,
    /// herdr.
    pub herdr: &'a mut dyn Surface,
    /// The plugin's `config.toml`.
    pub config: &'a mut ConfigFile,
    /// How long a tab label update waits for its lock.
    pub lock_wait: Duration,
    /// Now, Unix seconds (the ages `list` shows).
    pub now: i64,
}

/// Whether this module runs `cmd`.
pub fn handles(cmd: &str) -> bool {
    specs::command(cmd).is_some()
}

/// Runs subcommand `cmd` with `argv`, the words after it; the exit code.
pub fn run(cmd: &str, argv: &[String], io: &mut Io<'_>) -> i32 {
    let Some(spec) = specs::command(cmd) else {
        return 2;
    };
    let args = match parse(spec, argv) {
        Parsed::Run(args) => args,
        exit => return print_exit(exit, io),
    };
    if cmd == "open" {
        // the commands that need no database
        return open::open(&args, io).unwrap_or_else(|exit| print_exit(exit, io));
    }
    let store = match Store::open(io.db_path, BUSY_TIMEOUT) {
        Ok(store) => store,
        Err(OpenError::NotReady(_)) => return not_ready(io),
        Err(OpenError::Sqlite(_)) if !io.db_path.exists() => {
            // Python's Store() creates the file, finds it not migrated and says so to the
            // worker; this leaves creating it to the worker, which migrates it
            want(io.db_path, MIGRATION_REVISION, unix_now());
            return not_ready(io);
        }
        Err(OpenError::Sqlite(err)) => return failed(io, &err),
    };
    match dispatch(cmd, &args, &store, io) {
        Ok(code) => code,
        Err(err) => failed(io, &err),
    }
}

/// Prints a help text or a usage error; its exit code.
fn print_exit(exit: Parsed, io: &mut Io<'_>) -> i32 {
    let Parsed::Exit {
        code,
        stdout,
        stderr,
    } = exit
    else {
        return 2;
    };
    let _ = io.out.write_all(stdout.as_bytes());
    let _ = io.err.write_all(stderr.as_bytes());
    code
}

fn dispatch(cmd: &str, args: &Args, store: &Store, io: &mut Io<'_>) -> rusqlite::Result<i32> {
    match cmd {
        "list" => list::list(store, args.has("--all"), io),
        "answer" => answer::answer(store, args, io),
        "dismiss" => answer::dismiss(store, args, io),
        _ => Ok(2),
    }
}

/// The database lacks this code's schema (Python never creates it ready
/// either): start the worker, which migrates it, and say so.
fn not_ready(io: &mut Io<'_>) -> i32 {
    io.delivery.ensure_worker(None);
    let _ = writeln!(io.err, "{NOT_MIGRATED}");
    1
}

/// SQLite failed: Python's traceback, as one line.
fn failed(io: &mut Io<'_>, err: &dyn std::fmt::Display) -> i32 {
    let _ = writeln!(io.err, "inbox: {err}");
    1
}

/// Writes a warning to stderr as Python's logging does.
fn warner(err: &mut dyn Write) -> impl FnMut(&str) + '_ {
    move |msg: &str| {
        let _ = writeln!(err, "inbox: {msg}");
    }
}

//! What a successor is started as: the installed Rust worker under a root
//! (Python `rust_worker_binary`), else the Python worker of that root
//! (Python `python_worker_argv`, `start_successor`'s `uv run`).

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The Rust worker under a checkout, in the order looked at: a linked dev
/// checkout's `cargo build --release` first, then the release binary
/// `install-bin.sh` puts in `bin/`.
pub const RUST_WORKER_BINARIES: [&str; 2] = ["target/release/herdr-inbox", "bin/herdr-inbox"];

/// Python `python_worker_argv`'s code: the package under `argv[1]` on the
/// path, its CLI's `worker`.
pub const PYTHON_WORKER: &str = "import runpy, sys; sys.path.insert(0, sys.argv.pop(1)); \
     runpy.run_module('herdr_inbox.cli', run_name='__main__', alter_sys=True)";

/// The interpreter [`spawn_newer`](super::spawn_newer) runs the Python
/// worker with (Python runs its own `sys.executable`; a Rust worker has
/// none, so the `python3` on the spawn environment's `PATH`).
pub const PYTHON: &str = "python3";

/// The installed Rust worker under `root`: the first of
/// [`RUST_WORKER_BINARIES`] that is a regular file with an execute bit (a
/// symlink to one counts). `None`: no Rust worker there.
pub fn rust_worker_binary(root: &Path) -> Option<PathBuf> {
    // an execute bit stands in for Python's os.access(X_OK): this crate holds
    // no unsafe code to ask access(2), and the spawn's error covers the rest
    RUST_WORKER_BINARIES
        .iter()
        .map(|rel| root.join(rel))
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// The arguments after [`PYTHON`] that run the Python worker of the package
/// under `src`: `-I -c <code> <src> worker` (isolated: no `PYTHONPATH`
/// reaches it; the path goes in argv).
pub fn python_worker_args(src: &Path) -> Vec<OsString> {
    vec![
        "-I".into(),
        "-c".into(),
        PYTHON_WORKER.into(),
        src.as_os_str().to_owned(),
        "worker".into(),
    ]
}

/// The arguments after `uv` that run `root`'s Python worker in its own venv
/// (Python `start_successor`): `run --quiet --frozen --no-dev --project
/// <root> inbox worker`.
pub fn uv_worker_args(root: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["run", "--quiet", "--frozen", "--no-dev", "--project"]
        .map(OsString::from)
        .to_vec();
    args.extend([root.as_os_str().to_owned(), "inbox".into(), "worker".into()]);
    args
}

/// `worker`, the one argument of a Rust worker.
pub fn rust_worker_args() -> Vec<OsString> {
    vec![OsStr::new("worker").to_owned()]
}

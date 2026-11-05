//! Executables for tests to run, written by a child process.
//!
//! A test process that writes a file holds a write descriptor. A fork in any
//! other test thread inherits it until that child execs, and running the file
//! meanwhile fails with `Text file busy`. So no test thread ever writes one:
//! here the descriptor lives in a `sh` or `cp` that has exited before the call
//! returns. The bigger the file the likelier the race, and a real binary is
//! over 100 MB.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// `text` at `path`, executable. Its directory exists.
pub fn install(path: &Path, text: impl AsRef<[u8]>) {
    let mut sh = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    sh.stdin.take().unwrap().write_all(text.as_ref()).unwrap();
    assert!(sh.wait().unwrap().success(), "{}", path.display());
}

/// The executable `from`, also at `to`: a hard link, which writes nothing, or
/// across filesystems a copy.
pub fn link_or_copy(from: &Path, to: &Path) {
    if fs::hard_link(from, to).is_ok() {
        return;
    }
    let cp = Command::new("cp").arg(from).arg(to).status().unwrap();
    assert!(cp.success(), "{}", to.display());
}

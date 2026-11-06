//! Claude Code's own record of a process, `<config>/sessions/<pid>.json`
//! (herdr-panorama `docs/protocol/session-linking.md` §3 P): written at
//! startup, rewritten on every session change (`/clear`, `/resume`), removed
//! on exit. In a pane herdr says which session runs; in a core terminal this
//! is what does, since `CLAUDE_CODE_SESSION_ID` stays as it was at start.
//!
//! A file that is not there says nothing: the concurrent-sessions gate may be
//! off. A file that is there and fails any check is [`Record::Unusable`], and
//! the caller refuses: it may be mid-rewrite, a leftover of a reused pid, or
//! a kind this design does not reason about, and after `/clear` the
//! environment would name the wrong conversation.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use herdr_agent_kit::agent_procs::Pid;
use herdr_agent_kit::agent_session::SessionId;
use rustix::fs::{Mode, OFlags};
use serde_json::Value;

const MAX_BYTES: u64 = 16 * 1024;

/// What the record of a process says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// There is none.
    Missing,
    /// There is one, and it cannot be used.
    Unusable,
    /// It names this session.
    Names(String),
}

/// The record `<config>/sessions/<pid>.json` of this exact process.
pub fn session(config: &Path, pid: Pid, start: &str) -> Record {
    let path = config.join("sessions").join(format!("{pid}.json"));
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = match rustix::fs::open(&path, flags, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) if std::io::Error::from(e).kind() == ErrorKind::NotFound => return Record::Missing,
        Err(_) => return Record::Unusable,
    };
    read(File::from(fd), pid, start).map_or(Record::Unusable, Record::Names)
}

fn read(file: File, pid: Pid, start: &str) -> Option<String> {
    let meta = file.metadata().ok()?;
    let ours = meta.uid() == rustix::process::getuid().as_raw();
    if !meta.file_type().is_file() || !ours || meta.nlink() != 1 || meta.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    parse(&bytes, pid, start)
}

/// `pid` and `procStart` must be this process, and `kind` the plain interactive session.
fn parse(bytes: &[u8], pid: Pid, start: &str) -> Option<String> {
    let Value::Object(v) = serde_json::from_slice(bytes).ok()? else {
        return None;
    };
    let same = v.get("pid").and_then(Value::as_u64) == Some(u64::from(pid))
        && v.get("procStart").and_then(Value::as_str) == Some(start)
        && v.get("kind").and_then(Value::as_str) == Some("interactive");
    let id = v.get("sessionId").and_then(Value::as_str)?;
    (same && SessionId::parse(id).is_some()).then(|| id.to_owned())
}

#[cfg(test)]
mod tests;

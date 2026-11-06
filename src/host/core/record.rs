//! Claude Code's own record of a process, `<config>/sessions/<pid>.json`
//! (herdr-panorama `docs/protocol/session-linking.md` §3 P): written at
//! startup, rewritten on every session change (`/clear`, `/resume`), removed
//! on exit. In a pane herdr says which session runs; in a core terminal this
//! is what does, since `CLAUDE_CODE_SESSION_ID` stays as it was at start.
//!
//! Every check that fails means no record. A record left by an earlier
//! process with the pid has another start token.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use herdr_agent_kit::agent_procs::Pid;
use herdr_agent_kit::agent_session::SessionId;
use rustix::fs::{Mode, OFlags};
use serde_json::Value;

const MAX_BYTES: u64 = 16 * 1024;

/// The session `<config>/sessions/<pid>.json` names for this exact process.
pub fn session(config: &Path, pid: Pid, start: &str) -> Option<String> {
    let path = config.join("sessions").join(format!("{pid}.json"));
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = File::from(rustix::fs::open(&path, flags, Mode::empty()).ok()?);
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

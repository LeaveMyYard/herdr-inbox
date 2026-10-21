//! The ssh destination and the fixed command lines of a pull (Python
//! `remote.valid_target`, `ssh`'s and `call`'s refusals, `remote_command`,
//! `PROBE`). Data never goes into a command line: it goes on stdin.
//!
//! An unchecked target or command cannot reach ssh: the ssh closure takes a
//! [`SafeTarget`] and a [`FixedCommand`], and both are only made here, after
//! Python's checks (an option-like target such as `-oProxyCommand=…` would
//! run a command on the operator's machine).

use super::pyvalue::py_str_repr;
use crate::store::mirror::{Machine, safe_path};

/// The paths probe, run under `/bin/sh` whatever the remote login shell is (Python `PROBE`).
pub const PROBE: &str = "exec /bin/sh -c 'PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH\"; \
                         for b in uv herdr claude; do command -v \"$b\" || echo \"missing:$b\"; done'";

/// An ssh destination that cannot be read as an option or carry shell
/// syntax (Python `valid_target`: `[A-Za-z0-9_.@:\[\]-]{1,253}`, no leading `-`).
pub fn valid_target(target: &str) -> bool {
    !target.is_empty()
        && !target.starts_with('-')
        && target.len() <= 253
        && target.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'@' | b':' | b'[' | b']' | b'-')
        })
}

/// A target that passed [`valid_target`]: the only kind ssh is run on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeTarget(String);

impl SafeTarget {
    /// The target, checked; else Python `ssh`'s refusal text.
    pub fn for_ssh(target: Option<&str>) -> Result<SafeTarget, String> {
        match target {
            Some(t) if valid_target(t) => Ok(SafeTarget(t.to_string())),
            Some(t) => Err(format!("refusing ssh target {}", py_str_repr(t))),
            None => Err("refusing ssh target None".into()),
        }
    }

    /// The target, checked; else Python `call`'s refusal text.
    pub fn for_call(target: Option<&str>) -> Result<SafeTarget, String> {
        target
            .filter(|t| valid_target(t))
            .map(|t| SafeTarget(t.to_string()))
            .ok_or_else(|| "refusing that ssh target".into())
    }

    /// The destination.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A command line built here from constants and checked paths: the only
/// kind ssh runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedCommand(String);

impl FixedCommand {
    /// The paths probe.
    pub fn probe() -> FixedCommand {
        FixedCommand(PROBE.to_string())
    }

    /// `<herdr> plugin list --json`, for a probed herdr path (`None` if not safe).
    pub fn plugin_list(herdr: &str) -> Option<FixedCommand> {
        safe_path(herdr).then(|| FixedCommand(format!("{herdr} plugin list --json")))
    }

    /// `inbox <sub>` (export or apply) with `rec`'s cached paths, each checked
    /// again; else Python `remote_command`'s `bad <key>`.
    pub fn inbox(rec: &Machine, sub: &str) -> Result<FixedCommand, String> {
        let checked = |key: &str, v: &Option<String>| -> Result<String, String> {
            v.as_deref()
                .filter(|p| safe_path(p))
                .map(String::from)
                .ok_or_else(|| format!("bad {key}"))
        };
        let uv = checked("uv", &rec.uv)?;
        let herdr = checked("herdr", &rec.herdr)?;
        let root = checked("plugin_root", &rec.plugin_root)?;
        Ok(FixedCommand(format!(
            "env HERDR_BIN_PATH={herdr} {uv} run --quiet --frozen --no-dev --project {root} inbox {sub}"
        )))
    }

    /// The command line.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a pull calls to run ssh: `(target, command, stdin)`.
pub type Ssh<'a> =
    &'a mut dyn FnMut(&SafeTarget, &FixedCommand, Option<&str>) -> crate::store::mirror::CallResult;

#[cfg(test)]
mod tests;

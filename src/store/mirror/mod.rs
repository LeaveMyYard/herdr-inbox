//! The operator's mirror of each remote machine (Python `remote.py`, the
//! puller's side; docs/REMOTE.md): `remote_machines`, `remote_questions`,
//! `remote_alerts`, and the outbox rows they read. The SQL is Python's,
//! statement for statement. Time comes in as an argument (Python `_now()`),
//! the ssh call and the remote's paths as closures, and what Python then
//! notifies comes back as [`Notice`]s (before `herdr.clean`, which the caller
//! applies).

mod problems;
mod pull;
mod reads;

use std::time::SystemTime;

use rusqlite::Row;

pub use problems::{NOT_VERIFIED, Problem};
pub use pull::{CallResult, FULL_EVERY};
pub use reads::group_key;

use crate::clock::iso_seconds;
use crate::text::py_isspace;

/// One row of `remote_machines` (Python's `dict(row)`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Machine {
    pub machine_id: String,
    pub label: Option<String>,
    pub target: Option<String>,
    pub db_id: Option<String>,
    pub cursor_seq: Option<i64>,
    pub version: Option<String>,
    pub uv: Option<String>,
    pub herdr: Option<String>,
    pub plugin_root: Option<String>,
    pub pulls: i64,
    pub failures: i64,
    pub last_ok_at: Option<String>,
    pub last_error: Option<String>,
    pub last_try_at: Option<String>,
    pub backoff_until: Option<String>,
}

const MACHINE_COLUMNS: &str = "machine_id, label, target, db_id, cursor_seq, version, uv, herdr, plugin_root, \
                               pulls, failures, last_ok_at, last_error, last_try_at, backoff_until";

impl Machine {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Machine> {
        Ok(Machine {
            machine_id: r.get(0)?,
            label: r.get(1)?,
            target: r.get(2)?,
            db_id: r.get(3)?,
            cursor_seq: r.get(4)?,
            version: r.get(5)?,
            uv: r.get(6)?,
            herdr: r.get(7)?,
            plugin_root: r.get(8)?,
            pulls: r.get(9)?,
            failures: r.get(10)?,
            last_ok_at: r.get(11)?,
            last_error: r.get(12)?,
            last_try_at: r.get(13)?,
            backoff_until: r.get(14)?,
        })
    }

    /// Its label, else its id (Python `rec.get("label") or rec["machine_id"]`).
    pub fn name(&self) -> &str {
        self.label
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or(&self.machine_id)
    }
}

/// A notification Python sends after a pull (`_notify(title, body)`), before
/// `herdr.clean`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Its title.
    pub title: String,
    /// Its body.
    pub body: String,
}

/// Python `_iso(t)`.
pub fn iso(t: SystemTime) -> String {
    iso_seconds(t)
}

/// Python `service_first_line`: the first line of the stripped text, at most
/// 200 characters (`str.splitlines` boundaries).
pub fn first_line(text: &str) -> String {
    let stripped = text.trim_matches(py_isspace);
    let is_break = |c: char| {
        matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        )
    };
    stripped
        .split(is_break)
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests;
